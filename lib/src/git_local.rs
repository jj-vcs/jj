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
//! paths), plus in-process wire FETCH and PUSH for `git://` (anonymous
//! Git-daemon), `ssh://` (scp-style included), and `http(s)://` (smart-HTTP)
//! remotes, all backed by `grit-lib` rather than a `git` subprocess or `gix`
//! transport. Shallow/`depth` fetches over the wire paths (`git://`, `ssh://`,
//! `http(s)://`) are also handled in-process; only `file://` shallow fetches and
//! remotes carrying `--push-option` (and, for the local path, remotes with
//! receive hooks) stay on jj's existing subprocess path (see
//! [`crate::git_subprocess`]). The functions here
//! translate between jj's production types ([`RefSpec`], [`GitFetchStatus`] /
//! [`GitRefUpdates`], [`GitPushStats`], [`RefToPush`], `RemoteName`) and the
//! `grit-lib` `transfer` / `fetch` / `gc` APIs, so the orchestration in
//! [`crate::git`] can adopt them without otherwise changing shape.

use std::num::NonZeroU32;
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
use grit_lib::transport::SshTransport;
use grit_lib::transport::Transport as _;
use grit_lib::transport::http::ureq_client::UreqHttpClient;
use grit_lib::transport::http::http_fetch;
use grit_lib::config::ConfigSet;
use grit_lib::credentials::HelperCredentialProvider;
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

/// If `remote` is an `ssh://` (or scp-style `host:path`) remote, return its URL
/// string in the form `grit_lib::transport::parse_ssh_url` accepts. Returns
/// `None` for every other scheme, which stay on jj's subprocess / `gix`
/// transport path.
///
/// This is the `ssh` analogue of [`git_daemon_remote_url`]: it covers both the
/// FETCH ([`fetch_ssh`], protocol v2) and PUSH ([`push_ssh`], protocol v0/v1)
/// directions. grit-lib's `SshTransport` spawns `ssh` exactly as Git does
/// (`GIT_SSH_COMMAND` / `GIT_SSH` / `ssh`), so authentication is delegated to the
/// user's ssh configuration — no separate credential handling is needed here.
pub(crate) fn ssh_remote_url(
    remote: &gix::Remote,
    direction: gix::remote::Direction,
) -> Option<String> {
    let url = remote.url(direction)?;
    if url.scheme != gix::url::Scheme::Ssh {
        return None;
    }
    // `to_bstring` reproduces the canonical `ssh://[user@]host[:port]/path` form,
    // which `parse_ssh_url` parses (it also accepts scp-style, but gix normalizes
    // to the `ssh://` scheme form here); require valid UTF-8 (ssh URLs always
    // are).
    let s = url.to_bstring();
    s.to_str().ok().map(|s| s.to_owned())
}

/// If `remote` is an `http://` or `https://` (smart-HTTP) remote, return its URL
/// string in the form `grit-lib`'s smart-HTTP transport accepts
/// (`http[s]://host[:port]/path`). Returns `None` for every other scheme, which
/// stay on jj's subprocess / `gix` transport path.
///
/// This is the smart-HTTP analogue of [`git_daemon_remote_url`]: it covers both
/// the FETCH ([`fetch_http`], protocol v2) and PUSH ([`push_http`], protocol
/// v0/v1) directions.
pub(crate) fn http_remote_url(
    remote: &gix::Remote,
    direction: gix::remote::Direction,
) -> Option<String> {
    let url = remote.url(direction)?;
    if url.scheme != gix::url::Scheme::Https && url.scheme != gix::url::Scheme::Http {
        return None;
    }
    // `to_bstring` reproduces the canonical `http[s]://host[:port]/path` form,
    // which grit-lib's `http_fetch` / `push_http` consume directly; require valid
    // UTF-8 (HTTP URLs always are).
    let s = url.to_bstring();
    s.to_str().ok().map(|s| s.to_owned())
}

/// Emit a jj-side `[grit-net]` routing marker (gated by the same `GRIT_NET_DEBUG`
/// env var as grit-lib's own networking traces, so they interleave). Shows which
/// remote jj decided to route through grit-lib's in-process transports — the
/// "before" bookend around grit-lib's own connect/negotiate/done lines.
fn net_route(msg: impl FnOnce() -> String) {
    if grit_lib::net_trace::enabled() {
        grit_lib::net_trace::line(&format!("jj: {}", msg()));
    }
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
        // The local / file:// path copies the exact object closure and does not
        // graft, so `depth` (shallow) fetches stay on the subprocess path; the
        // dispatch in `crate::git` only routes file:// here when `depth.is_none()`.
        ..Default::default()
    };

    net_route(|| format!("fetch (file) {} via grit-lib", remote_git_dir.display()));
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
/// A `depth` (shallow) fetch is forwarded to grit-lib as a `deepen N` request;
/// `fetch_remote` reads the shallow-info section and updates the local `shallow`
/// file (both v0/v1 and v2).
pub(crate) fn fetch_git_daemon(
    local_git_dir: &Path,
    remote_url: &str,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    prune: bool,
    depth: Option<NonZeroU32>,
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
        // `depth` maps to grit-lib's shallow fetch (`deepen N` over the wire);
        // `fetch_remote` drives the v2 shallow-info handshake and writes the
        // local `shallow` file.
        depth: depth.map(NonZeroU32::get),
        ..Default::default()
    };

    // Connect to the daemon. Request protocol v2 explicitly (git's own default
    // for `git://` is v2-capable); grit-lib's daemon transport performs the v2
    // handshake and `fetch_remote` runs the v2 `ls-refs` + `command=fetch`
    // negotiation. A server that lacks v2 silently downgrades to v0/v1, which
    // `fetch_remote` also handles, so no explicit fallback is needed here.
    net_route(|| format!("fetch (git://) {remote_url} via grit-lib"));
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

/// Fetch from an `ssh://` (or scp-style) remote entirely in process, over an ssh
/// subprocess speaking the Git wire protocol, with no `git` subprocess for the
/// fetch logic and no `gix` transport.
///
/// This mirrors [`fetch_git_daemon`] but the connection is an ssh subprocess:
/// [`grit_lib::transport::SshTransport`] spawns `ssh <host> git-upload-pack
/// '<path>'` (resolving the ssh program from `GIT_SSH_COMMAND` / `GIT_SSH`, then
/// `ssh`, exactly as Git does), reads the advertisement, and
/// [`grit_lib::fetch::fetch_remote`] runs the want/have negotiation. The returned
/// [`FetchOutcome`] has the same shape as the other fetch paths, so
/// [`fetch_outcome_to_status`] converts it identically.
///
/// Protocol v2 is requested (git's default for ssh): the transport exports
/// `GIT_PROTOCOL=version=2` into the ssh environment, which OpenSSH forwards
/// (`SendEnv GIT_PROTOCOL`). A server that lacks v2 ignores it and returns the
/// classic v0/v1 advertisement, which `fetch_remote` also handles, so no explicit
/// fallback is needed here. A `depth` (shallow) fetch is forwarded as a
/// `deepen N` request and handled in-process by `fetch_remote`.
///
/// Authentication is whatever the user's ssh configuration provides (keys,
/// agent, `known_hosts`); there is no separate credential handling.
pub(crate) fn fetch_ssh(
    local_git_dir: &Path,
    remote_url: &str,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    prune: bool,
    depth: Option<NonZeroU32>,
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
        depth: depth.map(NonZeroU32::get),
        ..Default::default()
    };

    // Connect over ssh, requesting protocol v2 (git's default for ssh). The
    // transport handles spawning `ssh` per the user's environment; a server that
    // lacks v2 silently downgrades to v0/v1, which `fetch_remote` also handles.
    net_route(|| format!("fetch (ssh) {remote_url} via grit-lib"));
    let transport = SshTransport::new();
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

/// Fetch from an `http(s)://` (smart-HTTP) remote entirely in process, over
/// stateless-RPC HTTP requests, with no `git` subprocess and no `gix`
/// transport.
///
/// This mirrors [`fetch_git_daemon`] but drives the smart-HTTP wire protocol via
/// [`grit_lib::transport::http::http_fetch`]: a `GET info/refs?service=git-upload-pack`
/// discovery followed by `POST git-upload-pack` negotiation round(s). The
/// returned [`FetchOutcome`] has the same shape as the other fetch paths, so
/// [`fetch_outcome_to_status`] converts it identically.
///
/// Protocol v2 is requested via the `Git-Protocol: version=2` request header
/// ([`UreqHttpClient::with_git_protocol`]); a v2 server returns its v2 capability
/// advertisement and `http_fetch` runs the v2 `ls-refs` + `command=fetch`
/// negotiation. A server that lacks v2 ignores the header and returns the classic
/// v0/v1 advertisement, which `http_fetch` also handles, so no explicit fallback
/// is needed here. A `depth` (shallow) fetch is forwarded as a `deepen N` request
/// and handled in-process by `http_fetch` (both the v0/v1 stateless RPC and the
/// v2 multi-POST flow read the shallow-info section and update `shallow`).
///
/// Build a [`UreqHttpClient`] wired with a config-driven credential provider, so
/// HTTP basic auth on a `401` runs the repo's configured `credential.helper`
/// programs (e.g. `osxkeychain`) exactly as Git would.
///
/// The provider is grit-lib's [`HelperCredentialProvider`], built from the git
/// config cascade rooted at `local_git_dir` (system + global + repo-local, the
/// same layers Git reads). It is **non-interactive**: when no configured helper
/// can supply a usable username/password it surfaces a typed
/// [`grit_lib::error::Error::Auth`] rather than prompting on a TTY, so an
/// in-process fetch/push over an auth'd remote fails fast (the caller may then
/// fall back to the subprocess path) instead of hanging.
///
/// If the config cascade cannot be loaded (which should not happen for a valid
/// repo), the client is built without a provider — unauthenticated requests still
/// work, and an auth'd remote returns the same typed `Error::Auth`.
fn http_client_with_credentials(
    local_git_dir: &Path,
    git_protocol: Option<&str>,
) -> UreqHttpClient {
    // Build the client honoring the repo's HTTP request-shaping config
    // (`http.proxy`, `http.cookieFile` + `http.saveCookies`, `http.extraHeader`)
    // via `UreqHttpClient::from_config`, then wire the same config's
    // `credential.helper` programs for `401` basic auth. Both reuse one loaded
    // `ConfigSet` (system + global + repo-local, the layers Git reads).
    //
    // If the config cannot be loaded — or `from_config` rejects a setting we
    // cannot honor in-process (e.g. a SOCKS `http.proxy`) — fall back to a plain
    // client so unauthenticated remotes still work; an auth'd remote then returns
    // the same typed `Error::Auth`.
    let client = match ConfigSet::load(Some(local_git_dir), true) {
        Ok(config) => {
            let provider = HelperCredentialProvider::new(config.clone());
            match UreqHttpClient::from_config(&config) {
                Ok(c) => c.with_credential_provider(Box::new(provider)),
                Err(_) => UreqHttpClient::with_credentials(Box::new(provider)),
            }
        }
        Err(_) => UreqHttpClient::new(),
    };
    match git_protocol {
        Some(v) => client.with_git_protocol(v.to_owned()),
        None => client,
    }
}

/// Credentials: this builds a [`HelperCredentialProvider`] from the repo's git
/// config (so `credential.helper` programs such as `osxkeychain` are used) and
/// wires it into the [`UreqHttpClient`]. On a `401` the client fills/retries; if
/// no helper can supply credentials it fails with the typed
/// [`grit_lib::error::Error::Auth`] (non-interactive, never a hang), which the
/// dispatch in [`crate::git`] may treat as a signal to fall back to the
/// subprocess path. Unauthenticated remotes are unaffected.
pub(crate) fn fetch_http(
    local_git_dir: &Path,
    remote_url: &str,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    prune: bool,
    depth: Option<NonZeroU32>,
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
        depth: depth.map(NonZeroU32::get),
        ..Default::default()
    };

    // Request protocol v2 via the `Git-Protocol: version=2` header; `http_fetch`
    // dispatches on the version reported by the `info/refs` advertisement, so a
    // v0/v1-only server transparently downgrades. The client carries a
    // config-driven credential provider so auth'd remotes work in-process.
    net_route(|| format!("fetch (http) {remote_url} via grit-lib"));
    let client = http_client_with_credentials(local_git_dir, Some("version=2"));
    let mut progress = NoProgress;
    let outcome = http_fetch(&client, local_git_dir, remote_url, &opts, &mut progress)?;
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

    net_route(|| format!("push (file) {} via grit-lib", remote_git_dir.display()));
    let outcome = grit_lib::transfer::push_local(
        local_git_dir,
        remote_git_dir,
        &specs,
        &PushOptions::default(),
    )?;

    push_outcome_to_stats(local_git_dir, &outcome, fetch_refspecs)
}

/// Push to an anonymous `git://` (Git-daemon) remote entirely in process, over a
/// TCP pkt-line connection, with no `git` subprocess and no `gix` transport.
///
/// This mirrors [`push_local`] but drives the `git-receive-pack` wire protocol:
/// [`grit_lib::transport::GitDaemonTransport`] connects and reads the
/// receive-pack advertisement, then [`grit_lib::push::push_remote`] decides each
/// update against the advertised remote refs, builds the minimal pack, streams
/// it, and parses `report-status`. The [`grit_lib::transfer::PushOutcome`] is
/// translated back by the shared [`push_outcome_to_stats`] exactly like the local
/// path, including the post-push remote-tracking-ref update.
///
/// Protocol v0/v1 only (receive-pack has no v2 serve loop, and `push_remote`
/// rejects a v2 connection); connect with [`ConnectOptions::default`]
/// (`protocol_version = 0`). Unlike the local path there are no hooks to detect
/// locally: any `pre-receive` / `update` hooks run on the *server*, which can
/// reject the push (surfaced as a remote rejection).
///
/// `push_options` carries `--push-option` values: when non-empty, grit-lib
/// negotiates the receive-pack `push-options` capability and sends one
/// `push-option <value>` pkt-line per entry (the server exposes them to its hooks
/// via `GIT_PUSH_OPTION_*`). If the server does not advertise the capability the
/// push fails with the typed [`grit_lib::error::Error::PushOptionsUnsupported`]
/// (surfaced here as [`GitLocalError::Grit`]).
pub(crate) fn push_git_daemon(
    local_git_dir: &Path,
    remote_url: &str,
    references: &[RefToPush],
    fetch_refspecs: &[String],
    push_options: &[String],
) -> Result<GitPushStats, GitLocalError> {
    let mut specs: Vec<PushRefSpec> = Vec::with_capacity(references.len());
    for r in references {
        specs.push(ref_to_push_spec(r)?);
    }

    // receive-pack is v0/v1 only; `push_remote` rejects a v2 connection, so
    // connect with the default (protocol_version = 0).
    net_route(|| format!("push (git://) {remote_url} via grit-lib"));
    let transport = GitDaemonTransport::new();
    let mut conn = transport.connect(remote_url, Service::ReceivePack, &ConnectOptions::default())?;

    let mut progress = NoProgress;
    let outcome = grit_lib::push::push_remote(
        local_git_dir,
        conn.as_mut(),
        &specs,
        &push_options_with(push_options),
        &mut progress,
    )?;

    push_outcome_to_stats(local_git_dir, &outcome, fetch_refspecs)
}

/// Push to an `ssh://` (or scp-style) remote entirely in process, over an ssh
/// subprocess speaking `git-receive-pack`, with no `git` subprocess for the push
/// logic and no `gix` transport.
///
/// This mirrors [`push_git_daemon`] but the connection is an ssh subprocess:
/// [`grit_lib::transport::SshTransport`] spawns `ssh <host> git-receive-pack
/// '<path>'` (resolving the ssh program from the user's environment exactly as
/// Git does), and [`grit_lib::push::push_remote`] drives the receive-pack
/// exchange. The [`grit_lib::transfer::PushOutcome`] is translated back by the
/// shared [`push_outcome_to_stats`].
///
/// Protocol v0/v1 only (as for [`push_git_daemon`]); connect with the default
/// (`protocol_version = 0`) so the transport does not set `GIT_PROTOCOL`. Server
/// receive hooks run on the remote and may reject the push. Authentication is
/// whatever the user's ssh configuration provides. `push_options` is forwarded
/// over the wire exactly as in [`push_git_daemon`].
pub(crate) fn push_ssh(
    local_git_dir: &Path,
    remote_url: &str,
    references: &[RefToPush],
    fetch_refspecs: &[String],
    push_options: &[String],
) -> Result<GitPushStats, GitLocalError> {
    let mut specs: Vec<PushRefSpec> = Vec::with_capacity(references.len());
    for r in references {
        specs.push(ref_to_push_spec(r)?);
    }

    net_route(|| format!("push (ssh) {remote_url} via grit-lib"));
    let transport = SshTransport::new();
    let mut conn = transport.connect(remote_url, Service::ReceivePack, &ConnectOptions::default())?;

    let mut progress = NoProgress;
    let outcome = grit_lib::push::push_remote(
        local_git_dir,
        conn.as_mut(),
        &specs,
        &push_options_with(push_options),
        &mut progress,
    )?;

    push_outcome_to_stats(local_git_dir, &outcome, fetch_refspecs)
}

/// Push to an `http(s)://` (smart-HTTP) remote entirely in process, over
/// stateless-RPC HTTP requests, with no `git` subprocess and no `gix`
/// transport.
///
/// This mirrors [`push_local`] but drives the smart-HTTP wire protocol via
/// [`grit_lib::transport::http::SmartHttpTransport::push`] (which discovers the
/// receive-pack advertisement, decides each update, builds the command block +
/// pack, POSTs `git-receive-pack`, and parses `report-status`). The
/// [`grit_lib::transfer::PushOutcome`] is translated back into jj's
/// [`GitPushStats`] by the shared [`push_outcome_to_stats`], exactly like the
/// local path, including the post-push remote-tracking-ref update.
///
/// Protocol v0/v1 only (grit-lib does not yet implement a v2 `command=push`); a
/// v2 receive-pack advertisement is rejected by grit-lib. Remotes with
/// receive-hooks stay on the subprocess path (see the dispatch in [`crate::git`]).
/// HTTP basic auth is handled in-process: the client carries a config-driven
/// [`HelperCredentialProvider`], so a `401` triggers the configured
/// `credential.helper` programs; a remote whose credentials cannot be supplied
/// non-interactively fails with the typed [`grit_lib::error::Error::Auth`].
/// `push_options` is forwarded over the wire exactly as in [`push_git_daemon`].
pub(crate) fn push_http(
    local_git_dir: &Path,
    remote_url: &str,
    references: &[RefToPush],
    fetch_refspecs: &[String],
    push_options: &[String],
) -> Result<GitPushStats, GitLocalError> {
    let mut specs: Vec<PushRefSpec> = Vec::with_capacity(references.len());
    for r in references {
        specs.push(ref_to_push_spec(r)?);
    }

    // Receive-pack is protocol v0/v1, so no default `Git-Protocol` header. The
    // client carries a config-driven credential provider so auth'd remotes work
    // in-process.
    net_route(|| format!("push (http) {remote_url} via grit-lib"));
    let client = http_client_with_credentials(local_git_dir, None);
    let transport = grit_lib::transport::http::SmartHttpTransport::new(client);
    let mut progress = NoProgress;
    let outcome = transport.push(
        local_git_dir,
        remote_url,
        &specs,
        &push_options_with(push_options),
        &mut progress,
    )?;

    push_outcome_to_stats(local_git_dir, &outcome, fetch_refspecs)
}

/// Build a [`PushOptions`] carrying the given server-side `--push-option` values.
/// Shared by the wire push paths (`git://`, `ssh`, `http`) so the options are
/// forwarded identically; an empty slice yields the default (no options).
fn push_options_with(push_options: &[String]) -> PushOptions {
    PushOptions {
        push_options: push_options.to_vec(),
        ..PushOptions::default()
    }
}

/// Translate a grit-lib [`grit_lib::transfer::PushOutcome`] into jj's
/// [`GitPushStats`], and update the local clone's remote-tracking refs for the
/// pushed branches (only those a fetch refspec maps), exactly as Git does after a
/// push. Shared by [`push_local`] and [`push_http`] so both transports produce
/// identical jj-visible state.
fn push_outcome_to_stats(
    local_git_dir: &Path,
    outcome: &grit_lib::transfer::PushOutcome,
    fetch_refspecs: &[String],
) -> Result<GitPushStats, GitLocalError> {
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
