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

use std::collections::HashSet;
use std::error::Error as StdError;
use std::io;
#[cfg(test)]
use std::io::BufReader;
use std::io::Write as _;
use std::num::NonZeroU32;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use bstr::ByteSlice as _;
use gix::protocol::transport::client::TransportWithoutIO as _;
use itertools::Itertools as _;
use thiserror::Error;

use crate::git::FetchTagsOverride;
use crate::git::GitPushOptions;
use crate::git::GitPushStats;
use crate::git::GitSubprocessOptions;
use crate::git::NegativeRefSpec;
use crate::git::RefSpec;
use crate::git::RefToPush;
use crate::git_backend::GitBackend;
use crate::merge::Diff;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RefNameBuf;
use crate::ref_name::RemoteName;

/// Error originating by a Git subprocess
#[derive(Error, Debug)]
pub enum GitSubprocessError {
    #[error("Git process failed: {0}")]
    External(String),
}

fn external_error(err: impl StdError) -> GitSubprocessError {
    external_error_ref(&err)
}

fn external_error_ref(err: &(impl StdError + ?Sized)) -> GitSubprocessError {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(err) = source {
        message.push_str(": ");
        message.push_str(&err.to_string());
        source = err.source();
    }
    GitSubprocessError::External(message)
}

/// Context for creating Git subprocesses
pub(crate) struct GitSubprocessContext {
    git_dir: PathBuf,
}

impl GitSubprocessContext {
    pub(crate) fn new(git_dir: impl Into<PathBuf>, _options: GitSubprocessOptions) -> Self {
        Self { git_dir: git_dir.into() }
    }

    pub(crate) fn from_git_backend(
        git_backend: &GitBackend,
        options: GitSubprocessOptions,
    ) -> Self {
        Self::new(git_backend.git_repo_path(), options)
    }

    /// Perform a git fetch
    ///
    /// [`GitFetchStatus::NoRemoteRef`] is returned if ref doesn't exist. Note
    /// that `git` only returns one failed ref at a time.
    pub(crate) fn spawn_fetch(
        &self,
        remote_name: &RemoteName,
        refspecs: &[RefSpec],
        negative_refspecs: &[NegativeRefSpec],
        _callback: &mut dyn GitSubprocessCallback,
        depth: Option<NonZeroU32>,
        fetch_tags_override: Option<FetchTagsOverride>,
    ) -> Result<GitFetchStatus, GitSubprocessError> {
        if refspecs.is_empty() {
            return Ok(GitFetchStatus::Updates(GitRefUpdates::default()));
        }
        if !can_fetch_with_gix(refspecs) {
            return Err(GitSubprocessError::External(
                "unsupported fetch refspec shape".to_owned(),
            ));
        }
        if negative_refspecs
            .iter()
            .any(|refspec| refspec.to_git_format().contains('*'))
        {
            let expanded_refspecs = expand_negative_glob_fetch_refspecs(
                &self.git_dir,
                remote_name,
                refspecs,
                negative_refspecs,
            )?;
            return fetch_with_gix(
                &self.git_dir,
                remote_name,
                &expanded_refspecs,
                &[],
                depth,
                fetch_tags_override,
            );
        }
        fetch_with_gix(
            &self.git_dir,
            remote_name,
            refspecs,
            negative_refspecs,
            depth,
            fetch_tags_override,
        )
    }

    /// Prune particular branches
    pub(crate) fn spawn_branch_prune(
        &self,
        branches_to_prune: &[String],
    ) -> Result<(), GitSubprocessError> {
        if branches_to_prune.is_empty() {
            return Ok(());
        }
        tracing::debug!(?branches_to_prune, "pruning branches");
        for branch in branches_to_prune {
            let refname = format!("refs/remotes/{branch}");
            grit_lib::refs::delete_ref(&self.git_dir, &refname)
                .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        }
        Ok(())
    }

    /// Queries local remote for the default branch name.
    pub(crate) fn spawn_remote_show(
        &self,
        remote_name: &RemoteName,
    ) -> Result<Option<RefNameBuf>, GitSubprocessError> {
        remote_default_branch(&self.git_dir, remote_name).map(|branch| branch.map(Into::into))
    }

    /// Push references to git
    ///
    /// All pushes are forced, using --force-with-lease to perform a test&set
    /// operation on the remote repository
    ///
    /// Return tuple with
    ///     1. refs that failed to push
    ///     2. refs that succeeded to push
    pub(crate) fn spawn_push(
        &self,
        remote_name: &RemoteName,
        references: &[RefToPush],
        callback: &mut dyn GitSubprocessCallback,
        options: &GitPushOptions,
    ) -> Result<GitPushStats, GitSubprocessError> {
        if let Some(stats) =
            try_push_local(&self.git_dir, remote_name, references, callback, options)?
        {
            return Ok(stats);
        }

        let mut local_repo =
            gix::open(&self.git_dir).map_err(|err| GitSubprocessError::External(err.to_string()))?;
        local_repo
            .committer_or_set_generic_fallback()
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        let push_urls = receive_pack_urls(&self.git_dir, remote_name)?;
        if push_urls.is_empty() {
            return Err(GitSubprocessError::External(format!(
                "No URL configured for git remote '{}'",
                remote_name.as_str()
            )));
        }

        let mut stats = GitPushStats::default();
        for push_url in push_urls {
            let url_stats = if let Some(remote_git_dir) =
                local_remote_git_dir_from_path(local_remote_path_from_url(&self.git_dir, &push_url))
            {
                push_local_to_git_dir(
                    &local_repo,
                    remote_name,
                    references,
                    callback,
                    options,
                    &remote_git_dir,
                )?
            } else {
                send_receive_pack_commands(&local_repo, &push_url, references, callback, options)?
            };
            merge_push_stats(&mut stats, url_stats);
        }
        Ok(stats)
    }
}

struct RawGitObject<'a> {
    kind: gix::objs::Kind,
    data: &'a [u8],
}

impl gix::objs::WriteTo for RawGitObject<'_> {
    fn write_to(&self, out: &mut dyn io::Write) -> io::Result<()> {
        out.write_all(self.data)
    }

    fn kind(&self) -> gix::objs::Kind {
        self.kind
    }

    fn size(&self) -> u64 {
        self.data.len() as u64
    }
}

fn try_push_local(
    git_dir: &Path,
    remote_name: &RemoteName,
    references: &[RefToPush],
    callback: &mut dyn GitSubprocessCallback,
    options: &GitPushOptions,
) -> Result<Option<GitPushStats>, GitSubprocessError> {
    let Some(remote_git_dir) = local_remote_git_dir(git_dir, remote_name)? else {
        return Ok(None);
    };

    let mut local_repo =
        gix::open(git_dir).map_err(|err| GitSubprocessError::External(err.to_string()))?;
    local_repo
        .committer_or_set_generic_fallback()
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    push_local_to_git_dir(
        &local_repo,
        remote_name,
        references,
        callback,
        options,
        &remote_git_dir,
    )
    .map(Some)
}

fn push_local_to_git_dir(
    local_repo: &gix::Repository,
    remote_name: &RemoteName,
    references: &[RefToPush],
    callback: &mut dyn GitSubprocessCallback,
    options: &GitPushOptions,
    remote_git_dir: &Path,
) -> Result<GitPushStats, GitSubprocessError> {
    let mut remote_repo =
        gix::open(remote_git_dir).map_err(|err| GitSubprocessError::External(err.to_string()))?;
    remote_repo
        .committer_or_set_generic_fallback()
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;

    let mut stats = GitPushStats::default();
    let mut pushes = Vec::new();
    for reference in references {
        let destination = reference.refspec.destination();
        let source_id = reference
            .refspec
            .source()
            .map(|source| {
                gix::ObjectId::from_hex(source.as_bytes())
                    .map_err(|err| GitSubprocessError::External(err.to_string()))
            })
            .transpose()?;
        let current_target = remote_repo
            .try_find_reference(destination)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?
            .and_then(|reference| target_object_id(&reference.inner.target));
        let current_matches_expected =
            current_target.as_ref().map(|id| id.as_ref()) == reference.expected_location;
        let current_matches_source = source_id.is_some_and(|id| current_target == Some(id));
        if !current_matches_expected && !current_matches_source {
            stats
                .rejected
                .push((destination.into(), Some("stale info".to_owned())));
            continue;
        }

        if let Some(id) = source_id {
            copy_reachable_objects(&local_repo, &remote_repo, id)?;
        }
        let expected = match current_target {
            Some(id) => gix::refs::transaction::PreviousValue::MustExistAndMatch(id.into()),
            None => gix::refs::transaction::PreviousValue::MustNotExist,
        };
        let name = destination
            .try_into()
            .map_err(|err| GitSubprocessError::External(format!("invalid ref name: {err}")))?;
        let change = if let Some(id) = source_id {
            gix::refs::transaction::Change::Update {
                log: gix::refs::transaction::LogChange {
                    message: "push from jj".into(),
                    ..Default::default()
                },
                expected,
                new: id.into(),
            }
        } else {
            gix::refs::transaction::Change::Delete {
                expected,
                log: gix::refs::transaction::RefLog::AndReference,
            }
        };
        let edit = gix::refs::transaction::RefEdit {
            change,
            name,
            deref: false,
        };
        let mut local_tracking_edit = None;
        if let Some(branch_name) = destination.strip_prefix("refs/heads/") {
            let tracking_name = format!(
                "refs/remotes/{remote}/{branch_name}",
                remote = remote_name.as_str()
            );
            if local_repo
                .try_find_reference(&tracking_name)
                .map_err(|err| GitSubprocessError::External(err.to_string()))?
                .is_some()
                && remote_fetch_maps_branch(&local_repo, remote_name, branch_name, &tracking_name)?
            {
                let tracking_change = match source_id {
                    Some(id) => gix::refs::transaction::Change::Update {
                        log: gix::refs::transaction::LogChange {
                            message: "push from jj".into(),
                            ..Default::default()
                        },
                        expected: gix::refs::transaction::PreviousValue::Any,
                        new: id.into(),
                    },
                    None => gix::refs::transaction::Change::Delete {
                        expected: gix::refs::transaction::PreviousValue::Any,
                        log: gix::refs::transaction::RefLog::AndReference,
                    },
                };
                local_tracking_edit = Some(gix::refs::transaction::RefEdit {
                    change: tracking_change,
                    name: tracking_name.try_into().map_err(|err| {
                        GitSubprocessError::External(format!("invalid tracking ref name: {err}"))
                    })?,
                    deref: false,
                });
            }
        }
        pushes.push(PendingLocalPush {
            destination: destination.into(),
            old: current_target,
            new: source_id,
            edit,
            local_tracking_edit,
            remote_rejected: false,
        });
    }

    run_receive_pre_update_hooks(
        &remote_git_dir,
        &mut pushes,
        &options.remote_push_options,
        callback,
    )?;

    let mut edits = Vec::new();
    let mut local_tracking_edits = Vec::new();
    let mut accepted_updates = Vec::new();
    for push in pushes {
        if push.remote_rejected {
            stats
                .remote_rejected
                .push((push.destination, Some("hook declined".to_owned())));
            continue;
        }
        accepted_updates.push(AcceptedReceiveUpdate {
            destination: push.destination.clone(),
            old: push.old,
            new: push.new,
        });
        edits.push(push.edit);
        if let Some(edit) = push.local_tracking_edit {
            local_tracking_edits.push(edit);
        }
        stats.pushed.push(push.destination);
    }

    remote_repo
        .edit_references(edits)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    run_post_receive_hook(
        &remote_git_dir,
        &accepted_updates,
        &options.remote_push_options,
        callback,
    )?;
    local_repo
        .edit_references(local_tracking_edits)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    Ok(stats)
}

fn merge_push_stats(stats: &mut GitPushStats, new_stats: GitPushStats) {
    stats.pushed.extend(new_stats.pushed);
    stats.rejected.extend(new_stats.rejected);
    stats.remote_rejected.extend(new_stats.remote_rejected);
    stats
        .unexported_bookmarks
        .extend(new_stats.unexported_bookmarks);
    stats.pushed.sort();
    stats.pushed.dedup();
    stats.rejected.sort();
    stats.rejected.dedup();
    stats.remote_rejected.sort();
    stats.remote_rejected.dedup();
}

struct PendingLocalPush {
    destination: GitRefNameBuf,
    old: Option<gix::ObjectId>,
    new: Option<gix::ObjectId>,
    edit: gix::refs::transaction::RefEdit,
    local_tracking_edit: Option<gix::refs::transaction::RefEdit>,
    remote_rejected: bool,
}

struct AcceptedReceiveUpdate {
    destination: GitRefNameBuf,
    old: Option<gix::ObjectId>,
    new: Option<gix::ObjectId>,
}

fn run_receive_pre_update_hooks(
    remote_git_dir: &Path,
    pushes: &mut [PendingLocalPush],
    push_options: &[String],
    callback: &mut dyn GitSubprocessCallback,
) -> Result<(), GitSubprocessError> {
    let receive_input = receive_hook_input(pushes.iter().map(|push| {
        (
            push.destination.as_str(),
            push.old.as_ref(),
            push.new.as_ref(),
        )
    }));
    if !run_receive_hook(
        remote_git_dir,
        "pre-receive",
        &[],
        &receive_input,
        push_options,
        callback,
    )? {
        for push in pushes {
            push.remote_rejected = true;
        }
        return Ok(());
    }

    for push in pushes {
        let args = vec![
            push.destination.as_str().to_owned(),
            hook_object_id(push.old.as_ref()),
            hook_object_id(push.new.as_ref()),
        ];
        if !run_receive_hook(remote_git_dir, "update", &args, &[], push_options, callback)? {
            push.remote_rejected = true;
            let message = format!(
                "error: hook declined to update {}\n",
                push.destination.as_str()
            );
            callback
                .remote_sideband(message.as_bytes(), None)
                .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        }
    }
    Ok(())
}

fn run_post_receive_hook(
    remote_git_dir: &Path,
    accepted_updates: &[AcceptedReceiveUpdate],
    push_options: &[String],
    callback: &mut dyn GitSubprocessCallback,
) -> Result<(), GitSubprocessError> {
    let receive_input = receive_hook_input(accepted_updates.iter().map(|update| {
        (
            update.destination.as_str(),
            update.old.as_ref(),
            update.new.as_ref(),
        )
    }));
    run_receive_hook(
        remote_git_dir,
        "post-receive",
        &[],
        &receive_input,
        push_options,
        callback,
    )?;
    Ok(())
}

fn receive_hook_input<'a>(
    updates: impl IntoIterator<
        Item = (
            &'a str,
            Option<&'a gix::ObjectId>,
            Option<&'a gix::ObjectId>,
        ),
    >,
) -> Vec<u8> {
    let mut input = Vec::new();
    for (name, old, new) in updates {
        input.extend_from_slice(
            format!("{} {} {name}\n", hook_object_id(old), hook_object_id(new)).as_bytes(),
        );
    }
    input
}

fn hook_object_id(id: Option<&gix::ObjectId>) -> String {
    id.map(ToString::to_string)
        .unwrap_or_else(|| null_sha1().to_string())
}

fn run_receive_hook(
    remote_git_dir: &Path,
    name: &str,
    args: &[String],
    stdin: &[u8],
    push_options: &[String],
    callback: &mut dyn GitSubprocessCallback,
) -> Result<bool, GitSubprocessError> {
    let hook_path = remote_git_dir.join("hooks").join(name);
    if !hook_path.is_file() {
        return Ok(true);
    }

    let mut command = Command::new(&hook_path);
    command
        .current_dir(remote_git_dir)
        .env("GIT_DIR", remote_git_dir)
        .env("GIT_PUSH_OPTION_COUNT", push_options.len().to_string())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (i, option) in push_options.iter().enumerate() {
        command.env(format!("GIT_PUSH_OPTION_{i}"), option);
    }
    let mut child = command
        .spawn()
        .map_err(|err| GitSubprocessError::External(format!("failed to run {name} hook: {err}")))?;
    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin
            .write_all(stdin)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    emit_hook_output(callback, &output.stdout)?;
    emit_hook_output(callback, &output.stderr)?;
    Ok(output.status.success())
}

fn emit_hook_output(
    callback: &mut dyn GitSubprocessCallback,
    output: &[u8],
) -> Result<(), GitSubprocessError> {
    if !output.is_empty() {
        callback
            .remote_sideband(output, None)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    }
    Ok(())
}

fn remote_fetch_maps_branch(
    repo: &gix::Repository,
    remote_name: &RemoteName,
    branch_name: &str,
    tracking_name: &str,
) -> Result<bool, GitSubprocessError> {
    let remote = repo
        .find_remote(remote_name.as_str())
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    Ok(remote
        .refspecs(gix::remote::Direction::Fetch)
        .iter()
        .any(|refspec| {
            fetch_refspec_maps_branch(&refspec.to_ref().to_bstring(), branch_name, tracking_name)
        }))
}

fn fetch_refspec_maps_branch(refspec: &[u8], branch_name: &str, tracking_name: &str) -> bool {
    let Ok(refspec) = str::from_utf8(refspec) else {
        return false;
    };
    let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
    let Some((source, destination)) = refspec.split_once(':') else {
        return false;
    };
    let source_ref = format!("refs/heads/{branch_name}");
    if !matches_refspec_pattern(source, &source_ref) {
        return false;
    }
    let Some(captured) = capture_refspec_wildcard(source, &source_ref) else {
        return destination == tracking_name;
    };
    let mapped = destination.replacen('*', captured, 1);
    mapped == tracking_name
}

fn matches_refspec_pattern(pattern: &str, value: &str) -> bool {
    if let Some(captured) = capture_refspec_wildcard(pattern, value) {
        !captured.is_empty() || pattern.contains('*')
    } else {
        pattern == value
    }
}

fn capture_refspec_wildcard<'a>(pattern: &str, value: &'a str) -> Option<&'a str> {
    let (prefix, suffix) = pattern.split_once('*')?;
    value
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
}

fn local_remote_git_dir(
    git_dir: &Path,
    remote_name: &RemoteName,
) -> Result<Option<PathBuf>, GitSubprocessError> {
    let config = grit_lib::config::ConfigSet::load(Some(git_dir), true)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let remote_path = if push_urls_exist(&config, remote_name) {
        local_push_url(git_dir, &config, remote_name)
    } else {
        local_fetch_url(git_dir, &config, remote_name)
    };
    let Some(remote_path) = remote_path else {
        return Ok(None);
    };
    Ok(local_remote_git_dir_from_path(Some(remote_path)))
}

fn local_remote_git_dir_from_path(remote_path: Option<PathBuf>) -> Option<PathBuf> {
    let remote_path = remote_path?;
    Some(if remote_path.join(".git").is_dir() {
        remote_path.join(".git")
    } else {
        remote_path
    })
}

fn receive_pack_urls(
    git_dir: &Path,
    remote_name: &RemoteName,
) -> Result<Vec<String>, GitSubprocessError> {
    let config = grit_lib::config::ConfigSet::load(Some(git_dir), true)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    if push_urls_exist(&config, remote_name) {
        return Ok(config
            .get_all(&format!("remote.{}.pushurl", remote_name.as_str()))
            .into_iter()
            .collect());
    }
    Ok(config
        .get(&format!("remote.{}.url", remote_name.as_str()))
        .into_iter()
        .collect())
}

fn push_urls_exist(config: &grit_lib::config::ConfigSet, remote_name: &RemoteName) -> bool {
    !config
        .get_all(&format!("remote.{}.pushurl", remote_name.as_str()))
        .is_empty()
}

fn local_push_url(
    git_dir: &Path,
    config: &grit_lib::config::ConfigSet,
    remote_name: &RemoteName,
) -> Option<PathBuf> {
    let push_urls = config.get_all(&format!("remote.{}.pushurl", remote_name.as_str()));
    if push_urls.len() != 1 {
        return None;
    }
    push_urls
        .iter()
        .filter_map(|url| local_remote_path_from_url(git_dir, url))
        .exactly_one()
        .ok()
}

fn local_fetch_url(
    git_dir: &Path,
    config: &grit_lib::config::ConfigSet,
    remote_name: &RemoteName,
) -> Option<PathBuf> {
    let url = config.get(&format!("remote.{}.url", remote_name.as_str()))?;
    local_remote_path_from_url(git_dir, &url)
}

fn local_remote_path_from_url(git_dir: &Path, url: &str) -> Option<PathBuf> {
    if let Some(path) = local_path_from_file_url(url) {
        return Some(path);
    }
    if url.contains("://") || is_ssh_transport_url(url) {
        return None;
    }
    if let Some(path) = local_path_from_tilde_url(url) {
        return Some(path);
    }
    let path = PathBuf::from(url);
    if path.is_absolute() {
        return Some(path);
    }
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join(&path));
    }
    candidates.push(git_dir.join(&path));
    candidates.extend(
        git_dir
            .ancestors()
            .take(6)
            .map(|ancestor| ancestor.join(&path)),
    );
    candidates
        .into_iter()
        .filter(|candidate| candidate.exists())
        .unique()
        .exactly_one()
        .ok()
}

fn local_path_from_file_url(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    let path = path
        .strip_prefix("localhost/")
        .map(|path| format!("/{path}"))
        .unwrap_or_else(|| path.to_owned());
    let path = percent_decode_url_path(&path)?;
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

fn percent_decode_url_path(path: &str) -> Option<String> {
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next()?;
            let low = bytes.next()?;
            decoded.push(hex_digit(high)? << 4 | hex_digit(low)?);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).ok()
}

fn local_path_from_tilde_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("~/")?;
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(rest))
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_ssh_transport_url(url: &str) -> bool {
    if url.starts_with("ssh://") || url.starts_with("git+ssh://") {
        return true;
    }
    if url.contains("://") {
        return false;
    }
    let colon = url.find(':');
    let slash = url.find('/');
    colon.is_some_and(|colon| slash.is_none_or(|slash| colon < slash))
}

fn copy_reachable_objects(
    local_repo: &gix::Repository,
    remote_repo: &gix::Repository,
    id: gix::ObjectId,
) -> Result<(), GitSubprocessError> {
    let mut stack = vec![id];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if remote_repo
            .try_find_object(id)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?
            .is_some()
        {
            continue;
        }

        let object = local_repo
            .find_object(id)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        stack.extend(object_child_ids(&object)?);
        let written_id = remote_repo
            .write_object(RawGitObject {
                kind: object.kind,
                data: &object.data,
            })
            .map_err(|err| GitSubprocessError::External(err.to_string()))?
            .detach();
        if written_id != id {
            return Err(GitSubprocessError::External(format!(
                "copied object {id} but wrote {written_id}"
            )));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn pack_reachable_objects(
    local_repo: &gix::Repository,
    tips: impl IntoIterator<Item = gix::ObjectId>,
) -> Result<Vec<u8>, GitSubprocessError> {
    tracing::debug!("collecting reachable objects for receive-pack");
    let mut ids = collect_reachable_object_ids(local_repo, tips)?;
    tracing::debug!(object_count = ids.len(), "collected reachable objects for receive-pack");
    ids.sort_unstable();
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        let object = local_repo
            .find_object(id)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        let count = gix_pack::data::output::Count::from_data(id, None);
        entries.push(
            gix_pack::data::output::Entry::from_data(
                &count,
                &gix::objs::Data {
                    kind: object.kind,
                    data: &object.data,
                    object_hash: object.id.kind(),
                },
            )
            .map_err(|err| GitSubprocessError::External(err.to_string()))?,
        );
    }

    let num_entries = entries.len();
    let mut pack = Vec::new();
    let mut writer = gix_pack::data::output::bytes::FromEntriesIter::new(
        [Ok::<_, gix_pack::data::output::entry::Error>(entries)].into_iter(),
        &mut pack,
        u32::try_from(num_entries).map_err(|err| {
            GitSubprocessError::External(format!("too many objects to pack: {err}"))
        })?,
        gix_pack::data::Version::V2,
        gix::hash::Kind::Sha1,
    );
    while let Some(result) = writer.next() {
        result.map_err(|err| GitSubprocessError::External(err.to_string()))?;
    }
    tracing::debug!(bytes = pack.len(), "built receive-pack packfile");
    Ok(pack)
}

#[allow(dead_code)]
fn collect_reachable_object_ids(
    local_repo: &gix::Repository,
    tips: impl IntoIterator<Item = gix::ObjectId>,
) -> Result<Vec<gix::ObjectId>, GitSubprocessError> {
    let mut stack = tips.into_iter().collect_vec();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let object = local_repo
            .find_object(id)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        stack.extend(object_child_ids(&object)?);
    }
    Ok(seen.into_iter().collect())
}

fn object_child_ids(object: &gix::Object<'_>) -> Result<Vec<gix::ObjectId>, GitSubprocessError> {
    match object.kind {
        gix::objs::Kind::Commit => {
            let mut children = Vec::new();
            let mut commit = object.to_commit_ref_iter();
            children.push(
                commit
                    .tree_id()
                    .map_err(|err| GitSubprocessError::External(err.to_string()))?,
            );
            children.extend(object.to_commit_ref_iter().parent_ids());
            Ok(children)
        }
        gix::objs::Kind::Tree => gix::objs::TreeRefIter::from_bytes(&object.data, object.id.kind())
            .map(|entry| {
                entry
                    .map(|entry| entry.oid.to_owned())
                    .map_err(|err| GitSubprocessError::External(err.to_string()))
            })
            .collect(),
        gix::objs::Kind::Tag => {
            Ok(vec![object.to_tag_ref_iter().target_id().map_err(
                |err| GitSubprocessError::External(err.to_string()),
            )?])
        }
        gix::objs::Kind::Blob => Ok(Vec::new()),
    }
}

fn can_fetch_with_gix(refspecs: &[RefSpec]) -> bool {
    refspecs.iter().all(|refspec| {
        let git_refspec = refspec.to_git_format_not_forced();
        let Some((source, destination)) = git_refspec.split_once(':') else {
            return false;
        };
        let is_bookmark_refspec =
            source.starts_with("refs/heads/") && destination.starts_with("refs/remotes/");
        let is_tag_refspec =
            source.starts_with("refs/tags/") && destination.starts_with("refs/jj/remote-tags/");
        let source_wildcards = source.matches('*').count();
        let destination_wildcards = destination.matches('*').count();
        (is_bookmark_refspec || is_tag_refspec)
            && (source_wildcards == 0 && destination_wildcards == 0
                || source_wildcards == 1 && destination_wildcards == 1)
    })
}

fn expand_negative_glob_fetch_refspecs(
    git_dir: &Path,
    remote_name: &RemoteName,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
) -> Result<Vec<RefSpec>, GitSubprocessError> {
    let mut repo =
        gix::open(git_dir).map_err(|err| GitSubprocessError::External(err.to_string()))?;
    repo.committer_or_set_generic_fallback()
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let mut remote = repo
        .find_remote(remote_name.as_str())
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let gix_refspecs = refspecs
        .iter()
        .map(|refspec| gix::bstr::BString::from(refspec.to_git_format()))
        .collect_vec();
    remote
        .replace_refspecs(gix_refspecs.iter(), gix::remote::Direction::Fetch)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let mut progress = gix::progress::Discard;
    let ref_map = remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?
        .ref_map(&mut progress, Default::default())
        .map_err(|err| GitSubprocessError::External(err.to_string()))?
        .0;
    ref_map
        .mappings
        .into_iter()
        .filter_map(|mapping| {
            let source = mapping.remote.as_name()?.to_str().ok()?;
            if negative_refspecs
                .iter()
                .any(|refspec| refspec_matches(refspec.source(), source))
            {
                return None;
            }
            let destination = mapping.local?.to_str().ok()?.to_owned();
            Some(Ok(RefSpec::forced_fetch(source.to_owned(), destination)))
        })
        .try_collect()
}

fn refspec_matches(pattern: &str, refname: &str) -> bool {
    grit_lib::wildmatch::wildmatch(pattern.as_bytes(), refname.as_bytes(), 0)
}

fn fetch_with_gix(
    git_dir: &Path,
    remote_name: &RemoteName,
    refspecs: &[RefSpec],
    negative_refspecs: &[NegativeRefSpec],
    depth: Option<NonZeroU32>,
    fetch_tags_override: Option<FetchTagsOverride>,
) -> Result<GitFetchStatus, GitSubprocessError> {
    let mut repo =
        gix::open(git_dir).map_err(|err| GitSubprocessError::External(err.to_string()))?;
    repo.committer_or_set_generic_fallback()
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let mut remote = repo
        .find_remote(remote_name.as_str())
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let gix_refspecs = refspecs
        .iter()
        .map(|refspec| gix::bstr::BString::from(refspec.to_git_format()))
        .collect_vec();
    remote
        .replace_refspecs(gix_refspecs.iter(), gix::remote::Direction::Fetch)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    if let Some(fetch_tags_override) = fetch_tags_override {
        remote = remote.with_fetch_tags(match fetch_tags_override {
            FetchTagsOverride::AllTags => gix::remote::fetch::Tags::All,
            FetchTagsOverride::NoTags => gix::remote::fetch::Tags::None,
        });
    }

    let mut progress = gix::progress::Discard;
    let extra_refspecs = negative_refspecs
        .iter()
        .map(|refspec| {
            gix::refspec::parse(
                refspec.to_git_format().as_str().into(),
                gix::refspec::parse::Operation::Fetch,
            )
            .map(|refspec| refspec.to_owned())
            .map_err(|err| GitSubprocessError::External(err.to_string()))
        })
        .try_collect()?;
    let prepare = remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?
        .prepare_fetch(
            &mut progress,
            gix::remote::ref_map::Options {
                extra_refspecs,
                ..Default::default()
            },
        )
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    prune_wildcard_fetch_refs(git_dir, refspecs, prepare.ref_map())?;
    let mut receive = prepare.with_reflog_message(gix::remote::fetch::RefLogMessage::Prefixed {
        action: "fetch".to_owned(),
    });
    if let Some(depth) = depth {
        receive = receive.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(depth));
    }
    let outcome = match receive.receive(&mut progress, &gix::interrupt::IS_INTERRUPTED) {
        Ok(outcome) => outcome,
        Err(gix::remote::fetch::Error::NoMapping { refspecs, .. }) => {
            let source = refspecs
                .into_iter()
                .find_map(|refspec| {
                    refspec
                        .to_ref()
                        .source()
                        .and_then(|source| source.to_str().ok())
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            return Ok(GitFetchStatus::NoRemoteRef(source));
        }
        Err(err) => return Err(GitSubprocessError::External(err.to_string())),
    };

    Ok(GitFetchStatus::Updates(gix_fetch_updates(&outcome)))
}

fn prune_wildcard_fetch_refs(
    git_dir: &Path,
    refspecs: &[RefSpec],
    ref_map: &gix::remote::fetch::RefMap,
) -> Result<(), GitSubprocessError> {
    for refspec in refspecs {
        let git_refspec = refspec.to_git_format_not_forced();
        let Some((source, destination)) = git_refspec.split_once(':') else {
            continue;
        };
        if !source.ends_with("/*") || !destination.ends_with("/*") {
            continue;
        }
        let destination_prefix = destination.trim_end_matches('*').trim_end_matches('/');
        let remote_refs = ref_map
            .mappings
            .iter()
            .filter_map(|mapping| mapping.local.as_ref())
            .filter_map(|local| local.to_str().ok())
            .filter(|local| local.starts_with(destination_prefix))
            .collect::<std::collections::HashSet<_>>();
        let local_refs = grit_lib::refs::list_refs(git_dir, destination_prefix)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        for (local_ref, _) in local_refs {
            if !remote_refs.contains(local_ref.as_str()) {
                grit_lib::refs::delete_ref(git_dir, &local_ref)
                    .map_err(|err| GitSubprocessError::External(err.to_string()))?;
            }
        }
    }
    Ok(())
}

fn gix_fetch_updates(outcome: &gix::remote::fetch::Outcome) -> GitRefUpdates {
    let update_refs = match &outcome.status {
        gix::remote::fetch::Status::NoPackReceived { update_refs, .. }
        | gix::remote::fetch::Status::Change { update_refs, .. } => update_refs,
    };
    let mut updates = GitRefUpdates::default();
    for (update, mapping, _spec, edit) in update_refs.iter_mapping_updates(
        &outcome.ref_map.mappings,
        &outcome.ref_map.refspecs,
        &outcome.ref_map.extra_refspecs,
    ) {
        match update.mode {
            gix::remote::fetch::refs::update::Mode::NoChangeNeeded
            | gix::remote::fetch::refs::update::Mode::ImplicitTagNotSentByRemote => {}
            gix::remote::fetch::refs::update::Mode::FastForward
            | gix::remote::fetch::refs::update::Mode::Forced
            | gix::remote::fetch::refs::update::Mode::New => {
                if let Some((name, oid_diff)) = edit.and_then(fetch_ref_edit_diff) {
                    updates.updated.push((name, oid_diff));
                }
            }
            gix::remote::fetch::refs::update::Mode::RejectedSourceObjectNotFound { .. }
            | gix::remote::fetch::refs::update::Mode::RejectedTagUpdate
            | gix::remote::fetch::refs::update::Mode::RejectedNonFastForward
            | gix::remote::fetch::refs::update::Mode::RejectedToReplaceWithUnborn
            | gix::remote::fetch::refs::update::Mode::RejectedCurrentlyCheckedOut { .. } => {
                if let Some((name, oid_diff)) = edit
                    .and_then(fetch_ref_edit_diff)
                    .or_else(|| fetch_mapping_ref_diff(mapping))
                {
                    updates.rejected.push((name, oid_diff));
                }
            }
        }
    }
    updates
}

fn fetch_mapping_ref_diff(
    mapping: &gix::remote::fetch::refmap::Mapping,
) -> Option<(GitRefNameBuf, Diff<gix::ObjectId>)> {
    let name = mapping.local.as_ref()?.to_str().ok()?.to_owned().into();
    let after = mapping
        .remote
        .as_id()
        .map(ToOwned::to_owned)
        .unwrap_or_else(null_sha1);
    Some((name, Diff::new(null_sha1(), after)))
}

fn fetch_ref_edit_diff(
    edit: &gix::refs::transaction::RefEdit,
) -> Option<(GitRefNameBuf, Diff<gix::ObjectId>)> {
    let gix::refs::transaction::Change::Update { expected, new, .. } = &edit.change else {
        return None;
    };
    let before = previous_value_object_id(expected)?;
    let after = target_object_id(new)?;
    Some((
        edit.name.as_bstr().to_string().into(),
        Diff::new(before, after),
    ))
}

fn previous_value_object_id(
    value: &gix::refs::transaction::PreviousValue,
) -> Option<gix::ObjectId> {
    match value {
        gix::refs::transaction::PreviousValue::MustExistAndMatch(target)
        | gix::refs::transaction::PreviousValue::ExistingMustMatch(target) => {
            Some(target_object_id(target).unwrap_or_else(null_sha1))
        }
        gix::refs::transaction::PreviousValue::MustExist => None,
        gix::refs::transaction::PreviousValue::MustNotExist
        | gix::refs::transaction::PreviousValue::Any => Some(null_sha1()),
    }
}

fn target_object_id(target: &gix::refs::Target) -> Option<gix::ObjectId> {
    match target {
        gix::refs::Target::Object(id) => Some((*id).into()),
        gix::refs::Target::Symbolic(_) => None,
    }
}

fn null_sha1() -> gix::ObjectId {
    gix::ObjectId::null(gix::hash::Kind::Sha1)
}

fn remote_default_branch(
    git_dir: &Path,
    remote_name: &RemoteName,
) -> Result<Option<String>, GitSubprocessError> {
    let config = grit_lib::config::ConfigSet::load(Some(git_dir), true)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    if let Some(url) = config.get(&format!("remote.{}.url", remote_name.as_str())) {
        let remote_path = PathBuf::from(&url);
        if remote_path.is_absolute() {
            return local_remote_default_branch(&remote_path);
        }
    }
    remote_default_branch_with_gix(git_dir, remote_name)
}

fn local_remote_default_branch(remote_path: &Path) -> Result<Option<String>, GitSubprocessError> {
    let (remote_git_dir, remote_work_tree) = if remote_path.join(".git").is_dir() {
        (remote_path.join(".git"), Some(remote_path))
    } else {
        (remote_path.to_owned(), None)
    };
    let remote_repo = grit_lib::repo::Repository::open(&remote_git_dir, remote_work_tree)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let refs = grit_lib::ls_remote::ls_remote(
        &remote_repo.git_dir,
        &remote_repo.odb,
        &grit_lib::ls_remote::Options {
            symref: true,
            ..Default::default()
        },
    )
    .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let Some(head) = refs.iter().find(|entry| entry.name == "HEAD") else {
        return Ok(None);
    };
    if let Some(symref_target) = &head.symref_target {
        return Ok(symref_target.strip_prefix("refs/heads/").map(str::to_owned));
    }
    let head_oid = head.oid;
    Ok(refs.into_iter().find_map(|entry| {
        (entry.oid == head_oid)
            .then(|| entry.name.strip_prefix("refs/heads/").map(str::to_owned))
            .flatten()
    }))
}

fn remote_default_branch_with_gix(
    git_dir: &Path,
    remote_name: &RemoteName,
) -> Result<Option<String>, GitSubprocessError> {
    let repo = gix::open(git_dir).map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let remote = repo
        .find_remote(remote_name.as_str())
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let head_refspec = gix::refspec::parse("HEAD".into(), gix::refspec::parse::Operation::Fetch)
        .map(|refspec| refspec.to_owned())
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    let mut progress = gix::progress::Discard;
    let ref_map = remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?
        .ref_map(
            &mut progress,
            gix::remote::ref_map::Options {
                extra_refspecs: vec![head_refspec],
                ..Default::default()
            },
        )
        .map_err(|err| GitSubprocessError::External(err.to_string()))?
        .0;
    let mut head_object = None;
    for remote_ref in &ref_map.remote_refs {
        let target = match remote_ref {
            gix::protocol::handshake::Ref::Symbolic {
                full_ref_name,
                target,
                ..
            } if full_ref_name.as_slice() == b"HEAD" => target.as_bstr(),
            gix::protocol::handshake::Ref::Direct {
                full_ref_name,
                object,
            } if full_ref_name.as_slice() == b"HEAD" => {
                head_object = Some(*object);
                continue;
            }
            gix::protocol::handshake::Ref::Unborn { full_ref_name, .. }
                if full_ref_name.as_slice() == b"HEAD" =>
            {
                return Ok(None);
            }
            _ => continue,
        };
        return Ok(target
            .to_str()
            .ok()
            .and_then(|target| target.strip_prefix("refs/heads/").map(str::to_owned)));
    }
    if let Some(head_object) = head_object {
        return Ok(ref_map.remote_refs.into_iter().find_map(|remote_ref| {
            let (full_ref_name, object) = match remote_ref {
                gix::protocol::handshake::Ref::Direct {
                    full_ref_name,
                    object,
                }
                | gix::protocol::handshake::Ref::Symbolic {
                    full_ref_name,
                    object,
                    ..
                } => (full_ref_name, object),
                gix::protocol::handshake::Ref::Peeled { .. }
                | gix::protocol::handshake::Ref::Unborn { .. } => return None,
            };
            (object == head_object)
                .then(|| {
                    full_ref_name
                        .to_str()
                        .ok()
                        .and_then(|name| name.strip_prefix("refs/heads/").map(str::to_owned))
                })
                .flatten()
        }));
    }
    Ok(None)
}

/// Status of underlying `git fetch` operation.
#[derive(Clone, Debug)]
pub enum GitFetchStatus {
    /// Successfully fetched refs. There may be refs that couldn't be updated.
    Updates(GitRefUpdates),
    /// Fully-qualified ref that failed to fetch.
    ///
    /// Note that `git fetch` only returns one error at a time.
    NoRemoteRef(String),
}

/// Local changes made by `git fetch`.
#[derive(Clone, Debug, Default)]
pub struct GitRefUpdates {
    /// Git ref `(name, (old_oid, new_oid))`s that are successfully updated.
    ///
    /// `old_oid`/`new_oid` may be null or point to non-commit objects such as
    /// tags.
    pub updated: Vec<(GitRefNameBuf, Diff<gix::ObjectId>)>,
    /// Git ref `(name, (old_oid, new_oid)`s that are rejected or failed to
    /// update.
    pub rejected: Vec<(GitRefNameBuf, Diff<gix::ObjectId>)>,
}

#[allow(dead_code)]
fn parse_receive_pack_status(status: &[u8]) -> Result<GitPushStats, GitSubprocessError> {
    let mut lines = status.lines();
    let unpack_status = lines
        .next()
        .ok_or_else(|| GitSubprocessError::External("empty receive-pack status".to_owned()))?;
    let unpack_status = strip_pkt_line_prefix_bytes(unpack_status)
        .ok_or_else(|| GitSubprocessError::External("empty receive-pack status".to_owned()))?;
    let Some(unpack_result) = unpack_status.strip_prefix(b"unpack ") else {
        return Err(GitSubprocessError::External(format!(
            "receive-pack status missing unpack result: {}",
            unpack_status.to_str_lossy()
        )));
    };
    if unpack_result != b"ok" {
        return Err(GitSubprocessError::External(format!(
            "remote failed to unpack pushed objects: {}",
            unpack_result.to_str_lossy()
        )));
    }

    let mut stats = GitPushStats::default();
    for (idx, line) in lines.enumerate() {
        let Some(line) = strip_pkt_line_prefix_bytes(line) else {
            continue;
        };
        if line.is_empty() {
            continue;
        }
        if let Some(refname) = line.strip_prefix(b"ok ") {
            stats.pushed.push(parse_receive_pack_ref(idx, refname)?);
        } else if let Some(rest) = line.strip_prefix(b"ng ") {
            let (refname, reason) = rest.split_once_str(" ").ok_or_else(|| {
                GitSubprocessError::External(format!(
                    "receive-pack status line #{idx} rejected a ref without reason: {}",
                    line.to_str_lossy()
                ))
            })?;
            stats.remote_rejected.push((
                parse_receive_pack_ref(idx, refname)?,
                Some(reason.to_str_lossy().into_owned()),
            ));
        } else {
            return Err(GitSubprocessError::External(format!(
                "receive-pack status line #{idx} has unknown format: {}",
                line.to_str_lossy()
            )));
        }
    }
    Ok(stats)
}

#[allow(dead_code)]
fn parse_receive_pack_ref(idx: usize, refname: &[u8]) -> Result<GitRefNameBuf, GitSubprocessError> {
    let refname = refname.to_str().map_err(|err| {
        GitSubprocessError::External(format!(
            "receive-pack status line #{idx} has non-utf8 ref name {}: {err}",
            refname.to_str_lossy()
        ))
    })?;
    Ok(refname.into())
}

#[allow(dead_code)]
fn select_receive_pack_capabilities<'a>(
    advertised: &'a [&str],
    push_options: &[String],
) -> Result<Vec<&'a str>, GitSubprocessError> {
    let has = |capability: &str| advertised.contains(&capability);
    if !has("report-status") {
        return Err(GitSubprocessError::External(
            "remote did not advertise required receive-pack capability report-status".to_owned(),
        ));
    }

    let mut selected = vec!["report-status"];
    if has("side-band-64k") {
        selected.push("side-band-64k");
    }
    if !push_options.is_empty() {
        if !has("push-options") {
            return Err(GitSubprocessError::External(
                "remote does not support push options".to_owned(),
            ));
        }
        selected.push("push-options");
    }
    Ok(selected)
}

#[allow(dead_code)]
fn format_receive_pack_commands<'a>(
    updates: impl IntoIterator<Item = (Option<&'a gix::ObjectId>, Option<&'a gix::ObjectId>, &'a str)>,
    capabilities: &[&str],
) -> Vec<Vec<u8>> {
    let mut commands = Vec::new();
    for (index, (old, new, refname)) in updates.into_iter().enumerate() {
        let capabilities = (index == 0 && !capabilities.is_empty())
            .then(|| capabilities.join(" "));
        commands.push(format_receive_pack_command(old, new, refname, capabilities.as_deref()));
    }
    commands
}

#[allow(dead_code)]
fn format_receive_pack_commands_from_refs(
    references: &[RefToPush<'_>],
    capabilities: &[&str],
) -> Result<Vec<Vec<u8>>, GitSubprocessError> {
    let updates = references
        .iter()
        .map(|reference| {
            let new = reference
                .refspec
                .source()
                .map(|source| {
                    gix::ObjectId::from_hex(source.as_bytes())
                        .map_err(|err| GitSubprocessError::External(err.to_string()))
                })
                .transpose()?;
            Ok((
                reference.expected_location.map(Into::into),
                new,
                reference.refspec.destination(),
            ))
        })
        .collect::<Result<Vec<_>, GitSubprocessError>>()?;
    Ok(format_receive_pack_commands(
        updates
            .iter()
            .map(|(old, new, destination)| (old.as_ref(), new.as_ref(), *destination)),
        capabilities,
    ))
}

#[allow(dead_code)]
fn format_receive_pack_command(
    old: Option<&gix::ObjectId>,
    new: Option<&gix::ObjectId>,
    refname: &str,
    capabilities: Option<&str>,
) -> Vec<u8> {
    let mut command = format!(
        "{} {} {refname}",
        hook_object_id(old),
        hook_object_id(new)
    )
    .into_bytes();
    if let Some(capabilities) = capabilities {
        command.push(0);
        command.extend_from_slice(capabilities.as_bytes());
    }
    command
}

#[allow(dead_code)]
fn format_receive_pack_push_options(push_options: &[String]) -> Result<Vec<&[u8]>, GitSubprocessError> {
    push_options
        .iter()
        .map(|option| {
            if option.as_bytes().iter().any(|byte| matches!(byte, b'\0' | b'\n')) {
                return Err(GitSubprocessError::External(format!(
                    "push option contains unsupported NUL or LF byte: {option:?}"
                )));
            }
            Ok(option.as_bytes())
        })
        .collect()
}

fn send_receive_pack_commands(
    local_repo: &gix::Repository,
    remote_url: &str,
    references: &[RefToPush<'_>],
    callback: &mut dyn GitSubprocessCallback,
    options: &GitPushOptions,
) -> Result<GitPushStats, GitSubprocessError> {
    let parsed_url = gix::Url::try_from(remote_url)
        .map_err(external_error)?;
    let mut transport = gix::protocol::transport::client::blocking_io::connect::connect(
        parsed_url.clone(),
        gix::protocol::transport::client::blocking_io::connect::Options {
            version: gix::protocol::transport::Protocol::V1,
            ..Default::default()
        },
    )
    .map_err(external_error)?;
    if let Some(config) = local_repo
        .transport_options(remote_url.as_bytes().as_bstr(), None)
        .map_err(external_error)?
    {
        transport
            .configure(&*config)
            .map_err(|err| external_error_ref(&*err))?;
    }
    let (mut credential_helpers, _action_with_url, prompt_options) = local_repo
        .config_snapshot()
        .credential_helpers(parsed_url)
        .map_err(external_error)?;
    if credential_helpers.programs.is_empty() {
        credential_helpers
            .programs
            .extend(gix::credentials::helper::Cascade::platform_builtin());
    }
    let mut authenticate =
        move |action| credential_helpers.invoke(action, prompt_options.clone());
    let advertised_capabilities = {
        let mut progress = gix::progress::Discard;
        tracing::debug!(remote_url, "starting receive-pack handshake");
        let handshake = gix::protocol::handshake(
            &mut transport,
            gix::protocol::transport::Service::ReceivePack,
            &mut authenticate,
            Vec::new(),
            &mut progress,
        )
            .map_err(external_error)?;
        tracing::debug!("finished receive-pack handshake");
        handshake
            .capabilities
            .iter()
            .filter_map(|capability| capability.name().to_str().ok())
            .map(str::to_owned)
            .collect_vec()
    };
    let advertised_capabilities = advertised_capabilities
        .iter()
        .map(String::as_str)
        .collect_vec();
    let selected_capabilities =
        select_receive_pack_capabilities(&advertised_capabilities, &options.remote_push_options)?;
    let commands = format_receive_pack_commands_from_refs(references, &selected_capabilities)?;
    let push_options = format_receive_pack_push_options(&options.remote_push_options)?;
    let pack_tip_ids = receive_pack_tip_ids(references)?;
    let pack = (!pack_tip_ids.is_empty())
        .then(|| pack_reachable_objects(local_repo, pack_tip_ids))
        .transpose()?;
    tracing::debug!(
        pack_bytes = pack.as_ref().map_or(0, Vec::len),
        "prepared receive-pack request"
    );

    let mut writer = transport
        .request(
            gix::protocol::transport::client::WriteMode::OneLfTerminatedLinePerWriteCall,
            gix::protocol::transport::client::MessageKind::Flush,
            false,
        )
        .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    for command in commands {
        writer
            .write_all(&command)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
    }
    if !push_options.is_empty() {
        writer
            .write_message(gix::protocol::transport::client::MessageKind::Flush)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        for option in push_options {
            writer
                .write_all(option)
                .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        }
    }

    let mut reader = if let Some(pack) = pack {
        writer
            .write_message(gix::protocol::transport::client::MessageKind::Flush)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        let (mut raw_writer, reader) = writer.into_parts();
        raw_writer
            .write_all(&pack)
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        raw_writer
            .flush()
            .map_err(|err| GitSubprocessError::External(err.to_string()))?;
        drop(raw_writer);
        reader
    } else {
        writer
            .into_read()
            .map_err(|err| GitSubprocessError::External(err.to_string()))?
    };
    let mut status = String::new();
    let mut line = String::new();
    tracing::debug!("reading receive-pack status");
    while reader
        .readline_str(&mut line)
        .map_err(|err| GitSubprocessError::External(err.to_string()))?
        != 0
    {
        if let Some(line) = receive_pack_status_line(&line, callback) {
            status.push_str(line);
        }
        line.clear();
    }
    tracing::debug!(bytes = status.len(), "read receive-pack status");
    parse_receive_pack_status(status.as_bytes())
}

fn receive_pack_status_line<'a>(
    line: &'a str,
    callback: &mut dyn GitSubprocessCallback,
) -> Option<&'a str> {
    let line = strip_pkt_line_prefix(line)?;
    match line.as_bytes() {
        [1, rest @ ..] => str::from_utf8(rest).ok().and_then(strip_pkt_line_prefix),
        [2 | 3, rest @ ..] => {
            let rest = str::from_utf8(rest)
                .ok()
                .and_then(strip_pkt_line_prefix)
                .map(str::as_bytes)
                .unwrap_or(rest);
            let (body, term) = trim_sideband_line(rest);
            callback.remote_sideband(body, term).ok();
            None
        }
        _ => Some(line),
    }
}

fn strip_pkt_line_prefix(line: &str) -> Option<&str> {
    strip_pkt_line_prefix_bytes(line.as_bytes()).and_then(|line| str::from_utf8(line).ok())
}

fn strip_pkt_line_prefix_bytes(line: &[u8]) -> Option<&[u8]> {
    let Some(prefix) = line.get(..4) else {
        return Some(line);
    };
    if !prefix.iter().all(u8::is_ascii_hexdigit) {
        return Some(line);
    }
    (prefix != b"0000").then(|| &line[4..])
}

#[allow(dead_code)]
fn receive_pack_tip_ids(
    references: &[RefToPush<'_>],
) -> Result<Vec<gix::ObjectId>, GitSubprocessError> {
    references
        .iter()
        .filter_map(|reference| reference.refspec.source())
        .map(|source| {
            gix::ObjectId::from_hex(source.as_bytes())
                .map_err(|err| GitSubprocessError::External(err.to_string()))
        })
        .collect()
}

/// Handles Git command outputs.
pub trait GitSubprocessCallback {
    /// Whether to request progress information.
    fn needs_progress(&self) -> bool;

    /// Progress of local and remote operations.
    fn progress(&mut self, progress: &GitProgress) -> io::Result<()>;

    /// Single-line message that doesn't look like remote sideband or error.
    ///
    /// This may include authentication request from credential helpers.
    fn local_sideband(
        &mut self,
        message: &[u8],
        term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()>;

    /// Single-line sideband message received from remote.
    fn remote_sideband(
        &mut self,
        message: &[u8],
        term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()>;
}

/// Newline character that terminates sideband message line.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum GitSidebandLineTerminator {
    /// CR to remain on the same line.
    Cr = b'\r',
    /// LF to move to the next line.
    Lf = b'\n',
}

impl GitSidebandLineTerminator {
    /// Returns byte representation.
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Like `wait_with_output()`, but also emits sideband data through callback.
///
/// Git remotes can send custom messages on fetch and push, which the `git`
/// command prepends with `remote: `.
///
/// For instance, these messages can provide URLs to create Pull Requests
/// Progress of underlying `git` command operation.
#[derive(Clone, Debug, Default)]
pub struct GitProgress {
    /// `(frac, total)` of "Resolving deltas".
    pub deltas: (u64, u64),
    /// `(frac, total)` of "Receiving objects".
    pub objects: (u64, u64),
    /// `(frac, total)` of remote "Counting objects".
    pub counted_objects: (u64, u64),
    /// `(frac, total)` of remote "Compressing objects".
    pub compressed_objects: (u64, u64),
}

// TODO: maybe let callers print each field separately and remove overall()?
impl GitProgress {
    /// Overall progress normalized to 0 to 1 range.
    pub fn overall(&self) -> f32 {
        if self.total() != 0 {
            self.fraction() as f32 / self.total() as f32
        } else {
            0.0
        }
    }

    fn fraction(&self) -> u64 {
        self.objects.0 + self.deltas.0 + self.counted_objects.0 + self.compressed_objects.0
    }

    fn total(&self) -> u64 {
        self.objects.1 + self.deltas.1 + self.counted_objects.1 + self.compressed_objects.1
    }
}

/// Removes trailing spaces from sideband line, which may be padded by the `git`
/// CLI in order to clear the previous progress line.
fn trim_sideband_line(line: &[u8]) -> (&[u8], Option<GitSidebandLineTerminator>) {
    let (body, term) = match line {
        [body @ .., b'\r'] => (body, Some(GitSidebandLineTerminator::Cr)),
        [body @ .., b'\n'] => (body, Some(GitSidebandLineTerminator::Lf)),
        _ => (line, None),
    };
    let n = body.iter().rev().take_while(|&&b| b == b' ').count();
    (&body[..body.len() - n], term)
}

#[cfg(test)]
mod test {
    use bstr::BString;

    use super::*;

    #[derive(Debug, Default)]
    struct GitSubprocessCapture {
        progress: Vec<GitProgress>,
        local_sideband: Vec<BString>,
        remote_sideband: Vec<BString>,
    }

    impl GitSubprocessCallback for GitSubprocessCapture {
        fn needs_progress(&self) -> bool {
            true
        }

        fn progress(&mut self, progress: &GitProgress) -> io::Result<()> {
            self.progress.push(progress.clone());
            Ok(())
        }

        fn local_sideband(
            &mut self,
            message: &[u8],
            term: Option<GitSidebandLineTerminator>,
        ) -> io::Result<()> {
            self.local_sideband.push(message.into());
            if let Some(term) = term {
                self.local_sideband.push([term.as_byte()].into());
            }
            Ok(())
        }

        fn remote_sideband(
            &mut self,
            message: &[u8],
            term: Option<GitSidebandLineTerminator>,
        ) -> io::Result<()> {
            self.remote_sideband.push(message.into());
            if let Some(term) = term {
                self.remote_sideband.push([term.as_byte()].into());
            }
            Ok(())
        }
    }

    #[test]
    fn test_remote_default_branch() {
        let temp_dir = testutils::new_temp_dir();
        let local_repo = testutils::git::init_bare(temp_dir.path().join("local"));
        let remote_repo = testutils::git::init_bare(temp_dir.path().join("remote"));
        testutils::git::add_commit(
            &remote_repo,
            "refs/heads/main",
            "file",
            b"content",
            "initial",
            &[],
        );
        testutils::git::set_symbolic_reference(&remote_repo, "HEAD", "refs/heads/main");
        std::fs::write(
            local_repo.path().join("config"),
            format!(
                "[remote \"origin\"]\n\turl = {}\n",
                remote_repo.path().display()
            ),
        )
        .unwrap();

        assert_eq!(
            remote_default_branch(local_repo.path(), "origin".as_ref()).unwrap(),
            Some("main".to_string())
        );
    }

    #[test]
    fn test_remote_default_branch_unborn() {
        let temp_dir = testutils::new_temp_dir();
        let local_repo = testutils::git::init_bare(temp_dir.path().join("local"));
        let remote_repo = testutils::git::init_bare(temp_dir.path().join("remote"));
        std::fs::write(
            local_repo.path().join("config"),
            format!(
                "[remote \"origin\"]\n\turl = {}\n",
                remote_repo.path().display()
            ),
        )
        .unwrap();

        assert_eq!(
            remote_default_branch(local_repo.path(), "origin".as_ref()).unwrap(),
            None
        );
    }

    #[test]
    fn test_remote_default_branch_detached() {
        let temp_dir = testutils::new_temp_dir();
        let local_repo = testutils::git::init_bare(temp_dir.path().join("local"));
        let remote_repo = testutils::git::init_bare(temp_dir.path().join("remote"));
        let commit_id = testutils::git::add_commit(
            &remote_repo,
            "refs/heads/main",
            "file",
            b"content",
            "initial",
            &[],
        )
        .commit_id;
        testutils::git::set_head_to_id(&remote_repo, commit_id);
        std::fs::write(
            local_repo.path().join("config"),
            format!(
                "[remote \"origin\"]\n\turl = {}\n",
                remote_repo.path().display()
            ),
        )
        .unwrap();

        assert_eq!(
            remote_default_branch(local_repo.path(), "origin".as_ref()).unwrap(),
            Some("main".to_string())
        );
    }

    #[test]
    fn test_remote_default_branch_file_url() {
        let temp_dir = testutils::new_temp_dir();
        let local_repo = testutils::git::init_bare(temp_dir.path().join("local"));
        let remote_repo = testutils::git::init_bare(temp_dir.path().join("remote"));
        testutils::git::add_commit(
            &remote_repo,
            "refs/heads/main",
            "file",
            b"content",
            "initial",
            &[],
        );
        testutils::git::set_symbolic_reference(&remote_repo, "HEAD", "refs/heads/main");
        std::fs::write(
            local_repo.path().join("config"),
            format!(
                "[remote \"origin\"]\n\turl = file://{}\n",
                remote_repo.path().display()
            ),
        )
        .unwrap();

        assert_eq!(
            remote_default_branch(local_repo.path(), "origin".as_ref()).unwrap(),
            Some("main".to_string())
        );
    }

    #[test]
    fn test_parse_receive_pack_status() {
        let GitPushStats {
            pushed,
            rejected,
            remote_rejected,
            unexported_bookmarks,
        } = parse_receive_pack_status(
            b"unpack ok\nok refs/heads/main\nng refs/heads/rejected hook declined\n",
        )
        .unwrap();
        assert_eq!(pushed, ["refs/heads/main"].map(GitRefNameBuf::from));
        assert_eq!(rejected, []);
        assert_eq!(
            remote_rejected,
            [(
                GitRefNameBuf::from("refs/heads/rejected"),
                Some("hook declined".to_owned())
            )]
        );
        assert!(unexported_bookmarks.is_empty());

        let GitPushStats {
            pushed,
            rejected: _,
            remote_rejected: _,
            unexported_bookmarks: _,
        } = parse_receive_pack_status(b"000eunpack ok\n001cok refs/heads/main\n0000\n").unwrap();
        assert_eq!(pushed, ["refs/heads/main"].map(GitRefNameBuf::from));
    }

    #[test]
    fn test_receive_pack_status_line_strips_sideband() {
        let mut callback = GitSubprocessCapture::default();
        assert_eq!(
            receive_pack_status_line("\u{1}unpack ok\n", &mut callback),
            Some("unpack ok\n")
        );
        assert_eq!(
            receive_pack_status_line("000f\u{1}unpack ok\n", &mut callback),
            Some("unpack ok\n")
        );
        assert_eq!(
            receive_pack_status_line("\u{1}000eunpack ok\n", &mut callback),
            Some("unpack ok\n")
        );
        assert_eq!(receive_pack_status_line("0000", &mut callback), None);
        assert_eq!(
            receive_pack_status_line("\u{2}counting objects\n", &mut callback),
            None
        );
        assert_eq!(
            receive_pack_status_line("0015\u{2}writing objects\n", &mut callback),
            None
        );
        assert_eq!(
            receive_pack_status_line("\u{2}0015writing objects\n", &mut callback),
            None
        );
        assert_eq!(
            receive_pack_status_line("\u{3}fatal message\r", &mut callback),
            None
        );
        assert_eq!(
            receive_pack_status_line("ok refs/heads/main\n", &mut callback),
            Some("ok refs/heads/main\n")
        );
        assert_eq!(
            callback.remote_sideband,
            [
                "counting objects",
                "\n",
                "writing objects",
                "\n",
                "writing objects",
                "\n",
                "fatal message",
                "\r"
            ]
        );
    }

    #[test]
    fn test_parse_receive_pack_status_malformed() {
        assert!(parse_receive_pack_status(b"").is_err());
        assert!(parse_receive_pack_status(b"not-unpack ok\n").is_err());
        assert!(parse_receive_pack_status(b"unpack index-pack failed\n").is_err());
        assert!(parse_receive_pack_status(b"unpack ok\nng refs/heads/main\n").is_err());
        assert!(parse_receive_pack_status(b"unpack ok\nwat refs/heads/main\n").is_err());
    }

    #[test]
    fn test_select_receive_pack_capabilities() {
        assert_eq!(
            select_receive_pack_capabilities(&["report-status", "side-band-64k"], &[]).unwrap(),
            vec!["report-status", "side-band-64k"]
        );
        assert_eq!(
            select_receive_pack_capabilities(
                &["report-status", "side-band-64k", "push-options"],
                &["ci.skip".to_owned()],
            )
            .unwrap(),
            vec!["report-status", "side-band-64k", "push-options"]
        );
        assert!(select_receive_pack_capabilities(&["side-band-64k"], &[]).is_err());
        assert!(
            select_receive_pack_capabilities(&["report-status"], &["ci.skip".to_owned()])
                .is_err()
        );
    }

    #[test]
    fn test_format_receive_pack_commands() {
        let old = gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
        let new = gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222").unwrap();
        let delete = gix::ObjectId::from_hex(b"3333333333333333333333333333333333333333").unwrap();
        let commands = format_receive_pack_commands(
            [
                (Some(&old), Some(&new), "refs/heads/main"),
                (None, Some(&new), "refs/heads/new"),
                (Some(&delete), None, "refs/heads/delete"),
            ],
            &["report-status", "side-band-64k", "push-options"],
        );

        assert_eq!(
            commands,
            vec![
                b"1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/heads/main\0report-status side-band-64k push-options".to_vec(),
                b"0000000000000000000000000000000000000000 2222222222222222222222222222222222222222 refs/heads/new".to_vec(),
                b"3333333333333333333333333333333333333333 0000000000000000000000000000000000000000 refs/heads/delete".to_vec(),
            ]
        );
    }

    #[test]
    fn test_format_receive_pack_commands_from_refs() {
        let old = gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
        let new = gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222").unwrap();
        let delete = gix::ObjectId::from_hex(b"3333333333333333333333333333333333333333").unwrap();
        let update_refspec = RefSpec::forced_push(new.to_string(), "refs/heads/main");
        let create_refspec = RefSpec::forced_push(new.to_string(), "refs/heads/new");
        let delete_refspec = RefSpec::delete_push("refs/heads/delete");
        let refs = [
            RefToPush {
                refspec: &update_refspec,
                expected_location: Some(old.as_ref()),
            },
            RefToPush {
                refspec: &create_refspec,
                expected_location: None,
            },
            RefToPush {
                refspec: &delete_refspec,
                expected_location: Some(delete.as_ref()),
            },
        ];

        let commands =
            format_receive_pack_commands_from_refs(&refs, &["report-status", "side-band-64k"])
                .unwrap();
        assert_eq!(
            commands,
            vec![
                b"1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/heads/main\0report-status side-band-64k".to_vec(),
                b"0000000000000000000000000000000000000000 2222222222222222222222222222222222222222 refs/heads/new".to_vec(),
                b"3333333333333333333333333333333333333333 0000000000000000000000000000000000000000 refs/heads/delete".to_vec(),
            ]
        );
    }

    #[test]
    fn test_format_receive_pack_push_options() {
        let options = ["merge_request.create".to_owned(), "ci.skip".to_owned()];
        let formatted = format_receive_pack_push_options(&options).unwrap();
        assert_eq!(formatted, vec![b"merge_request.create".as_slice(), b"ci.skip"]);

        assert!(format_receive_pack_push_options(&["bad\noption".to_owned()]).is_err());
        assert!(format_receive_pack_push_options(&["bad\0option".to_owned()]).is_err());
    }

    #[test]
    fn test_receive_pack_tip_ids_ignores_deletes() {
        let id = gix::ObjectId::from_hex(b"2222222222222222222222222222222222222222").unwrap();
        let push_refspec = RefSpec::forced_push(id.to_string(), "refs/heads/main");
        let delete_refspec = RefSpec::delete_push("refs/heads/delete");
        let refs = [
            RefToPush {
                refspec: &push_refspec,
                expected_location: None,
            },
            RefToPush {
                refspec: &delete_refspec,
                expected_location: None,
            },
        ];

        assert_eq!(receive_pack_tip_ids(&refs).unwrap(), vec![id]);
    }

    #[test]
    fn test_receive_pack_urls_prefers_pushurls() {
        let temp_dir = tempfile::tempdir().unwrap();
        let git_dir = temp_dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(
            git_dir.join("config"),
            "\
[remote \"origin\"]
    url = https://example.com/fetch.git
    pushurl = ssh://example.com/first.git
    pushurl = ssh://example.com/second.git
",
        )
        .unwrap();

        assert_eq!(
            receive_pack_urls(&git_dir, &RemoteName::new("origin")).unwrap(),
            vec![
                "ssh://example.com/first.git".to_owned(),
                "ssh://example.com/second.git".to_owned()
            ]
        );
    }

    #[test]
    fn test_pack_reachable_objects_writes_valid_pack() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = gix::init(temp_dir.path()).unwrap();
        let blob_id = repo
            .write_object(RawGitObject {
                kind: gix::objs::Kind::Blob,
                data: b"hello",
            })
            .unwrap()
            .detach();

        let pack = pack_reachable_objects(&repo, [blob_id]).unwrap();
        assert!(pack.starts_with(b"PACK"));

        let entries = gix_pack::data::input::BytesToEntriesIter::new_from_header(
            BufReader::new(pack.as_slice()),
            gix_pack::data::input::Mode::Verify,
            gix_pack::data::input::EntryDataMode::Keep,
            gix::hash::Kind::Sha1,
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].header, gix_pack::data::entry::Header::Blob);
        assert_eq!(entries[0].decompressed_size, 5);
        assert_eq!(
            entries[0].trailer.as_ref().map(|id| id.as_slice()),
            Some(&pack[pack.len() - 20..])
        );
    }

    #[test]
    fn test_initial_overall_progress_is_zero() {
        assert_eq!(GitProgress::default().overall(), 0.0);
    }
}
