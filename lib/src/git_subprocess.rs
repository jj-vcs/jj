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

use std::io;
use std::io::BufReader;
use std::io::Read;
use std::num::NonZeroU32;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;

use bstr::ByteSlice as _;
use itertools::Itertools as _;
use thiserror::Error;

use crate::git::FetchTagsOverride;
use crate::git::GitPushStats;
use crate::git::NegativeRefSpec;
use crate::git::Progress;
use crate::git::RefSpec;
use crate::git::RefToPush;
use crate::git::RemoteCallbacks;
use crate::git_backend::GitBackend;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RefNameBuf;
use crate::ref_name::RemoteName;

// This is not the minimum required version, that would be 2.29.0, which
// introduced the `--no-write-fetch-head` option. However, that by itself
// is quite old and unsupported, so we don't want to encourage users to
// update to that.
//
// 2.40 still receives security patches (latest one was in Jan/2025)
const MINIMUM_GIT_VERSION: &str = "2.40.4";

/// Error originating by a Git subprocess
#[derive(Error, Debug)]
pub enum GitSubprocessError {
    #[error("Could not find repository at '{0}'")]
    NoSuchRepository(String),
    #[error("Could not execute the git process, found in the OS path '{path}'")]
    SpawnInPath {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("Could not execute git process at specified path '{path}'")]
    Spawn {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("Failed to wait for the git process")]
    Wait(std::io::Error),
    #[error(
        "Git does not recognize required option: {0} (note: supported version is \
         {MINIMUM_GIT_VERSION})"
    )]
    UnsupportedGitOption(String),
    #[error("Git process failed: {0}")]
    External(String),
}

/// Context for creating Git subprocesses
pub(crate) struct GitSubprocessContext<'a> {
    git_dir: PathBuf,
    git_executable_path: &'a Path,
}

impl<'a> GitSubprocessContext<'a> {
    pub(crate) fn new(git_dir: impl Into<PathBuf>, git_executable_path: &'a Path) -> Self {
        Self {
            git_dir: git_dir.into(),
            git_executable_path,
        }
    }

    pub(crate) fn from_git_backend(
        git_backend: &GitBackend,
        git_executable_path: &'a Path,
    ) -> Self {
        Self::new(git_backend.git_repo_path(), git_executable_path)
    }

    /// Create the Git command
    fn create_command(&self) -> Command {
        let mut git_cmd = Command::new(self.git_executable_path);
        // Hide console window on Windows (https://stackoverflow.com/a/60958956)
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            git_cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // TODO: here we are passing the full path to the git_dir, which can lead to UNC
        // bugs in Windows. The ideal way to do this is to pass the workspace
        // root to Command::current_dir and then pass a relative path to the git
        // dir
        git_cmd
            // The gitconfig-controlled automated spawning of the macOS `fsmonitor--daemon`
            // can cause strange behavior with certain subprocess operations.
            // For example: https://github.com/jj-vcs/jj/issues/6440.
            //
            // Nothing we're doing in `jj` interacts with this daemon, so we force the
            // config to be false for subprocess operations in order to avoid these
            // interactions.
            //
            // In a colocated workspace, the daemon will still get started the first
            // time a `git` command is run manually if the gitconfigs are set up that way.
            .args(["-c", "core.fsmonitor=false"])
            // Avoids an error message when fetching repos with submodules if
            // user has `submodule.recurse` configured to true in their Git
            // config (#7565).
            .args(["-c", "submodule.recurse=false"])
            .arg("--git-dir")
            .arg(&self.git_dir)
            // Disable translation and other locale-dependent behavior so we can
            // parse the output. LC_ALL precedes LC_* and LANG.
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::piped());

        git_cmd
    }

    /// Spawn the git command
    fn spawn_cmd(&self, mut git_cmd: Command) -> Result<Child, GitSubprocessError> {
        tracing::debug!(cmd = ?git_cmd, "spawning a git subprocess");
        git_cmd.spawn().map_err(|error| {
            if self.git_executable_path.is_absolute() {
                GitSubprocessError::Spawn {
                    path: self.git_executable_path.to_path_buf(),
                    error,
                }
            } else {
                GitSubprocessError::SpawnInPath {
                    path: self.git_executable_path.to_path_buf(),
                    error,
                }
            }
        })
    }

    /// Perform a git fetch
    ///
    /// This returns a fully qualified ref that wasn't fetched successfully
    /// Note that git only returns one failed ref at a time
    pub(crate) fn spawn_fetch(
        &self,
        remote_name: &RemoteName,
        refspecs: &[RefSpec],
        negative_refspecs: &[NegativeRefSpec],
        callbacks: &mut RemoteCallbacks<'_>,
        depth: Option<NonZeroU32>,
        fetch_tags_override: Option<FetchTagsOverride>,
    ) -> Result<Option<String>, GitSubprocessError> {
        if refspecs.is_empty() {
            return Ok(None);
        }
        let mut command = self.create_command();
        command.stdout(Stdio::piped());
        // attempt to prune stale refs with --prune
        // --no-write-fetch-head ensures our request is invisible to other parties
        command.args(["fetch", "--prune", "--no-write-fetch-head"]);
        if callbacks.progress.is_some() {
            command.arg("--progress");
        }
        if let Some(d) = depth {
            command.arg(format!("--depth={d}"));
        }
        match fetch_tags_override {
            Some(FetchTagsOverride::AllTags) => {
                command.arg("--tags");
            }
            Some(FetchTagsOverride::NoTags) => {
                command.arg("--no-tags");
            }
            None => {}
        }
        command.arg("--").arg(remote_name.as_str());
        command.args(
            refspecs
                .iter()
                .map(|x| x.to_git_format())
                .chain(negative_refspecs.iter().map(|x| x.to_git_format())),
        );

        let output = wait_with_progress(self.spawn_cmd(command)?, callbacks)?;

        parse_git_fetch_output(output)
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
        let mut command = self.create_command();
        command.stdout(Stdio::null());
        command.args(["branch", "--remotes", "--delete", "--"]);
        command.args(branches_to_prune);

        let output = wait_with_output(self.spawn_cmd(command)?)?;

        // we name the type to make sure that it is not meant to be used
        let () = parse_git_branch_prune_output(output)?;

        Ok(())
    }

    /// How we retrieve the remote's default branch:
    ///
    /// `git remote show <remote_name>`
    ///
    /// dumps a lot of information about the remote, with a line such as:
    /// `  HEAD branch: <default_branch>`
    pub(crate) fn spawn_remote_show(
        &self,
        remote_name: &RemoteName,
    ) -> Result<Option<RefNameBuf>, GitSubprocessError> {
        let mut command = self.create_command();
        command.stdout(Stdio::piped());
        command.args(["remote", "show", "--", remote_name.as_str()]);
        let output = wait_with_output(self.spawn_cmd(command)?)?;

        let output = parse_git_remote_show_output(output)?;

        // find the HEAD branch line in the output
        let maybe_branch = parse_git_remote_show_default_branch(&output.stdout)?;
        Ok(maybe_branch.map(Into::into))
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
        callbacks: &mut RemoteCallbacks<'_>,
    ) -> Result<GitPushStats, GitSubprocessError> {
        let mut command = self.create_command();
        command.stdout(Stdio::piped());
        // Currently jj does not support commit hooks, so we prevent git from running
        // them
        //
        // https://github.com/jj-vcs/jj/issues/3577 and https://github.com/jj-vcs/jj/issues/405
        // offer more context
        command.args(["push", "--porcelain", "--no-verify"]);
        if callbacks.progress.is_some() {
            command.arg("--progress");
        }
        command.args(
            references
                .iter()
                .map(|reference| format!("--force-with-lease={}", reference.to_git_lease())),
        );
        command.args(["--", remote_name.as_str()]);
        // with --force-with-lease we cannot have the forced refspec,
        // as it ignores the lease
        command.args(
            references
                .iter()
                .map(|r| r.refspec.to_git_format_not_forced()),
        );

        let output = wait_with_progress(self.spawn_cmd(command)?, callbacks)?;

        parse_git_push_output(output)
    }
}

/// Generate a GitSubprocessError::ExternalGitError if the stderr output was not
/// recognizable
fn external_git_error(stderr: &[u8]) -> GitSubprocessError {
    GitSubprocessError::External(format!(
        "External git program failed:\n{}",
        stderr.to_str_lossy()
    ))
}

/// Parse no such remote errors output from git
///
/// Returns the remote that wasn't found
///
/// To say this, git prints out a lot of things, but the first line is of the
/// form:
/// `fatal: '<remote>' does not appear to be a git repository`
/// or
/// `fatal: '<remote>': Could not resolve host: invalid-remote
fn parse_no_such_remote(stderr: &[u8]) -> Option<String> {
    let first_line = stderr.lines().next()?;
    let suffix = first_line
        .strip_prefix(b"fatal: '")
        .or_else(|| first_line.strip_prefix(b"fatal: unable to access '"))?;

    suffix
        .strip_suffix(b"' does not appear to be a git repository")
        .or_else(|| suffix.strip_suffix(b"': Could not resolve host: invalid-remote"))
        .map(|remote| remote.to_str_lossy().into_owned())
}

/// Parse error from refspec not present on the remote
///
/// This returns
///     Some(local_ref) that wasn't found by the remote
///     None if this wasn't the error
///
/// On git fetch even though --prune is specified, if a particular
/// refspec is asked for but not present in the remote, git will error out.
///
/// Git only reports one of these errors at a time, so we only look at the first
/// line
///
/// The first line is of the form:
/// `fatal: couldn't find remote ref refs/heads/<ref>`
fn parse_no_remote_ref(stderr: &[u8]) -> Option<String> {
    let first_line = stderr.lines().next()?;
    first_line
        .strip_prefix(b"fatal: couldn't find remote ref ")
        .map(|refname| refname.to_str_lossy().into_owned())
}

/// Parse remote tracking branch not found
///
/// This returns true if the error was detected
///
/// if a branch is asked for but is not present, jj will detect it post-hoc
/// so, we want to ignore these particular errors with git
///
/// The first line is of the form:
/// `error: remote-tracking branch '<branch>' not found`
fn parse_no_remote_tracking_branch(stderr: &[u8]) -> Option<String> {
    let first_line = stderr.lines().next()?;

    let suffix = first_line.strip_prefix(b"error: remote-tracking branch '")?;

    suffix
        .strip_suffix(b"' not found.")
        .or_else(|| suffix.strip_suffix(b"' not found"))
        .map(|branch| branch.to_str_lossy().into_owned())
}

/// Parse unknown options
///
/// Return the unknown option
///
/// If a user is running a very old git version, our commands may fail
/// We want to give a good error in this case
fn parse_unknown_option(stderr: &[u8]) -> Option<String> {
    let first_line = stderr.lines().next()?;
    first_line
        .strip_prefix(b"unknown option: --")
        .or(first_line
            .strip_prefix(b"error: unknown option `")
            .and_then(|s| s.strip_suffix(b"'")))
        .map(|s| s.to_str_lossy().into())
}

// return the fully qualified ref that failed to fetch
//
// note that git fetch only returns one error at a time
fn parse_git_fetch_output(output: Output) -> Result<Option<String>, GitSubprocessError> {
    if output.status.success() {
        return Ok(None);
    }

    // There are some git errors we want to parse out
    if let Some(option) = parse_unknown_option(&output.stderr) {
        return Err(GitSubprocessError::UnsupportedGitOption(option));
    }

    if let Some(remote) = parse_no_such_remote(&output.stderr) {
        return Err(GitSubprocessError::NoSuchRepository(remote));
    }

    if let Some(refspec) = parse_no_remote_ref(&output.stderr) {
        return Ok(Some(refspec));
    }

    if parse_no_remote_tracking_branch(&output.stderr).is_some() {
        return Ok(None);
    }

    Err(external_git_error(&output.stderr))
}

fn parse_git_branch_prune_output(output: Output) -> Result<(), GitSubprocessError> {
    if output.status.success() {
        return Ok(());
    }

    // There are some git errors we want to parse out
    if let Some(option) = parse_unknown_option(&output.stderr) {
        return Err(GitSubprocessError::UnsupportedGitOption(option));
    }

    if parse_no_remote_tracking_branch(&output.stderr).is_some() {
        return Ok(());
    }

    Err(external_git_error(&output.stderr))
}

fn parse_git_remote_show_output(output: Output) -> Result<Output, GitSubprocessError> {
    if output.status.success() {
        return Ok(output);
    }

    // There are some git errors we want to parse out
    if let Some(option) = parse_unknown_option(&output.stderr) {
        return Err(GitSubprocessError::UnsupportedGitOption(option));
    }

    if let Some(remote) = parse_no_such_remote(&output.stderr) {
        return Err(GitSubprocessError::NoSuchRepository(remote));
    }

    Err(external_git_error(&output.stderr))
}

fn parse_git_remote_show_default_branch(
    stdout: &[u8],
) -> Result<Option<String>, GitSubprocessError> {
    stdout
        .lines()
        .map(|x| x.trim())
        .find(|x| x.starts_with_str("HEAD branch:"))
        .inspect(|x| tracing::debug!(line = ?x.to_str_lossy(), "default branch"))
        .and_then(|x| x.split_str(" ").last().map(|y| y.trim()))
        .filter(|branch_name| branch_name != b"(unknown)")
        .map(|branch_name| branch_name.to_str())
        .transpose()
        .map_err(|e| GitSubprocessError::External(format!("git remote output is not utf-8: {e:?}")))
        .map(|b| b.map(|x| x.to_string()))
}

// git-push porcelain has the following format (per line)
// `<flag>\t<from>:<to>\t<summary> (<reason>)`
//
// <flag> is one of:
//     ' ' for a successfully pushed fast-forward;
//      + for a successful forced update
//      - for a successfully deleted ref
//      * for a successfully pushed new ref
//      !  for a ref that was rejected or failed to push; and
//      =  for a ref that was up to date and did not need pushing.
//
// <from>:<to> is the refspec
//
// <summary> is extra info (commit ranges or reason for rejected)
//
// <reason> is a human-readable explanation
fn parse_ref_pushes(stdout: &[u8]) -> Result<GitPushStats, GitSubprocessError> {
    if !stdout.starts_with(b"To ") {
        return Err(GitSubprocessError::External(format!(
            "Git push output unfamiliar:\n{}",
            stdout.to_str_lossy()
        )));
    }

    let mut push_stats = GitPushStats::default();
    for (idx, line) in stdout
        .lines()
        .skip(1)
        .take_while(|line| line != b"Done")
        .enumerate()
    {
        tracing::debug!("response #{idx}: {}", line.to_str_lossy());
        let [flag, reference, summary] = line.split_str("\t").collect_array().ok_or_else(|| {
            GitSubprocessError::External(format!(
                "Line #{idx} of git-push has unknown format: {}",
                line.to_str_lossy()
            ))
        })?;
        let full_refspec = reference
            .to_str()
            .map_err(|e| {
                format!(
                    "Line #{} of git-push has non-utf8 refspec {}: {}",
                    idx,
                    reference.to_str_lossy(),
                    e
                )
            })
            .map_err(GitSubprocessError::External)?;

        let reference: GitRefNameBuf = full_refspec
            .split_once(':')
            .map(|(_refname, reference)| reference.into())
            .ok_or_else(|| {
                GitSubprocessError::External(format!(
                    "Line #{idx} of git-push has full refspec without named ref: {full_refspec}"
                ))
            })?;

        match flag {
            // ' ' for a successfully pushed fast-forward;
            //  + for a successful forced update
            //  - for a successfully deleted ref
            //  * for a successfully pushed new ref
            //  =  for a ref that was up to date and did not need pushing.
            b"+" | b"-" | b"*" | b"=" | b" " => {
                push_stats.pushed.push(reference);
            }
            // ! for a ref that was rejected or failed to push; and
            b"!" => {
                if let Some(reason) = summary.strip_prefix(b"[remote rejected]") {
                    let reason = reason
                        .strip_prefix(b" (")
                        .and_then(|r| r.strip_suffix(b")"))
                        .map(|x| x.to_str_lossy().into_owned());
                    push_stats.remote_rejected.push((reference, reason));
                } else {
                    let reason = summary
                        .split_once_str("]")
                        .and_then(|(_, reason)| reason.strip_prefix(b" ("))
                        .and_then(|r| r.strip_suffix(b")"))
                        .map(|x| x.to_str_lossy().into_owned());
                    push_stats.rejected.push((reference, reason));
                }
            }
            unknown => {
                return Err(GitSubprocessError::External(format!(
                    "Line #{} of git-push starts with an unknown flag '{}': '{}'",
                    idx,
                    unknown.to_str_lossy(),
                    line.to_str_lossy()
                )));
            }
        }
    }

    Ok(push_stats)
}

// on Ok, return a tuple with
//  1. list of failed references from test and set
//  2. list of successful references pushed
fn parse_git_push_output(output: Output) -> Result<GitPushStats, GitSubprocessError> {
    if output.status.success() {
        let ref_pushes = parse_ref_pushes(&output.stdout)?;
        return Ok(ref_pushes);
    }

    if let Some(option) = parse_unknown_option(&output.stderr) {
        return Err(GitSubprocessError::UnsupportedGitOption(option));
    }

    if let Some(remote) = parse_no_such_remote(&output.stderr) {
        return Err(GitSubprocessError::NoSuchRepository(remote));
    }

    if output
        .stderr
        .lines()
        .any(|line| line.starts_with(b"error: failed to push some refs to "))
    {
        parse_ref_pushes(&output.stdout)
    } else {
        Err(external_git_error(&output.stderr))
    }
}

fn wait_with_output(child: Child) -> Result<Output, GitSubprocessError> {
    child.wait_with_output().map_err(GitSubprocessError::Wait)
}

/// Like `wait_with_output()`, but also emits sideband data through callback.
///
/// Git remotes can send custom messages on fetch and push, which the `git`
/// command prepends with `remote: `.
///
/// For instance, these messages can provide URLs to create Pull Requests
/// e.g.:
/// ```ignore
/// $ jj git push -c @
/// [...]
/// remote:
/// remote: Create a pull request for 'branch' on GitHub by visiting:
/// remote:      https://github.com/user/repo/pull/new/branch
/// remote:
/// ```
///
/// The returned `stderr` content does not include sideband messages.
fn wait_with_progress(
    mut child: Child,
    callbacks: &mut RemoteCallbacks<'_>,
) -> Result<Output, GitSubprocessError> {
    let (stdout, stderr) = thread::scope(|s| -> io::Result<_> {
        drop(child.stdin.take());
        let mut child_stdout = child.stdout.take().expect("stdout should be piped");
        let mut child_stderr = child.stderr.take().expect("stderr should be piped");
        let thread = s.spawn(move || -> io::Result<_> {
            let mut buf = Vec::new();
            child_stdout.read_to_end(&mut buf)?;
            Ok(buf)
        });
        let stderr = read_to_end_with_progress(&mut child_stderr, callbacks)?;
        let stdout = thread.join().expect("reader thread wouldn't panic")?;
        Ok((stdout, stderr))
    })
    .map_err(GitSubprocessError::Wait)?;
    let status = child.wait().map_err(GitSubprocessError::Wait)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[derive(Default)]
struct GitProgress {
    // (frac, total)
    deltas: (u64, u64),
    objects: (u64, u64),
    counted_objects: (u64, u64),
    compressed_objects: (u64, u64),
}

impl GitProgress {
    fn to_progress(&self) -> Progress {
        Progress {
            bytes_downloaded: None,
            overall: if self.total() != 0 {
                self.fraction() as f32 / self.total() as f32
            } else {
                0.0
            },
        }
    }

    fn fraction(&self) -> u64 {
        self.objects.0 + self.deltas.0 + self.counted_objects.0 + self.compressed_objects.0
    }

    fn total(&self) -> u64 {
        self.objects.1 + self.deltas.1 + self.counted_objects.1 + self.compressed_objects.1
    }
}

fn read_to_end_with_progress<R: Read>(
    src: R,
    callbacks: &mut RemoteCallbacks<'_>,
) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(src);
    let mut data = Vec::new();
    let mut git_progress = GitProgress::default();

    loop {
        // progress sent through sideband channel may be terminated by \r
        let start = data.len();
        read_until_cr_or_lf(&mut reader, &mut data)?;
        let line = &data[start..];
        if line.is_empty() {
            break;
        }

        if update_progress(line, &mut git_progress.objects, b"Receiving objects:")
            || update_progress(line, &mut git_progress.deltas, b"Resolving deltas:")
            || update_progress(
                line,
                &mut git_progress.counted_objects,
                b"remote: Counting objects:",
            )
            || update_progress(
                line,
                &mut git_progress.compressed_objects,
                b"remote: Compressing objects:",
            )
        {
            if let Some(cb) = callbacks.progress.as_mut() {
                cb(&git_progress.to_progress());
            }
            data.truncate(start);
        } else if let Some(message) = line.strip_prefix(b"remote: ") {
            if let Some(cb) = callbacks.sideband_progress.as_mut() {
                let (body, term) = trim_sideband_line(message);
                cb(body);
                if let Some(term) = term {
                    cb(&[term]);
                }
            }
            data.truncate(start);
        }
    }
    Ok(data)
}

fn update_progress(line: &[u8], progress: &mut (u64, u64), prefix: &[u8]) -> bool {
    if let Some(line) = line.strip_prefix(prefix) {
        if let Some((frac, total)) = read_progress_line(line) {
            *progress = (frac, total);
        }

        true
    } else {
        false
    }
}

fn read_until_cr_or_lf<R: io::BufRead + ?Sized>(
    reader: &mut R,
    dest_buf: &mut Vec<u8>,
) -> io::Result<()> {
    loop {
        let data = match reader.fill_buf() {
            Ok(data) => data,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        let (n, found) = match data.iter().position(|&b| matches!(b, b'\r' | b'\n')) {
            Some(i) => (i + 1, true),
            None => (data.len(), false),
        };

        dest_buf.extend_from_slice(&data[..n]);
        reader.consume(n);

        if found || n == 0 {
            return Ok(());
        }
    }
}

/// Read progress lines of the form: `<text> (<frac>/<total>)`
/// Ensures that frac < total
fn read_progress_line(line: &[u8]) -> Option<(u64, u64)> {
    // isolate the part between parenthesis
    let (_prefix, suffix) = line.split_once_str("(")?;
    let (fraction, _suffix) = suffix.split_once_str(")")?;

    // split over the '/'
    let (frac_str, total_str) = fraction.split_once_str("/")?;

    // parse to integers
    let frac = frac_str.to_str().ok()?.parse().ok()?;
    let total = total_str.to_str().ok()?.parse().ok()?;
    (frac <= total).then_some((frac, total))
}

/// Removes trailing spaces from sideband line, which may be padded by the `git`
/// CLI in order to clear the previous progress line.
fn trim_sideband_line(line: &[u8]) -> (&[u8], Option<u8>) {
    let (body, term) = match line {
        [body @ .., term @ (b'\r' | b'\n')] => (body, Some(*term)),
        _ => (line, None),
    };
    let n = body.iter().rev().take_while(|&&b| b == b' ').count();
    (&body[..body.len() - n], term)
}

#[cfg(test)]
mod test {
    use indoc::formatdoc;

    use super::*;

    const SAMPLE_NO_SUCH_REPOSITORY_ERROR: &[u8] =
        br###"fatal: unable to access 'origin': Could not resolve host: invalid-remote
fatal: Could not read from remote repository.

Please make sure you have the correct access rights
and the repository exists. "###;
    const SAMPLE_NO_SUCH_REMOTE_ERROR: &[u8] =
        br###"fatal: 'origin' does not appear to be a git repository
fatal: Could not read from remote repository.

Please make sure you have the correct access rights
and the repository exists. "###;
    const SAMPLE_NO_REMOTE_REF_ERROR: &[u8] = b"fatal: couldn't find remote ref refs/heads/noexist";
    const SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR: &[u8] =
        b"error: remote-tracking branch 'bookmark' not found";
    const SAMPLE_PUSH_REFS_PORCELAIN_OUTPUT: &[u8] = b"To origin
*\tdeadbeef:refs/heads/bookmark1\t[new branch]
+\tdeadbeef:refs/heads/bookmark2\tabcd..dead
-\tdeadbeef:refs/heads/bookmark3\t[deleted branch]
 \tdeadbeef:refs/heads/bookmark4\tabcd..dead
=\tdeadbeef:refs/heads/bookmark5\tabcd..abcd
!\tdeadbeef:refs/heads/bookmark6\t[rejected] (failure lease)
!\tdeadbeef:refs/heads/bookmark7\t[rejected]
!\tdeadbeef:refs/heads/bookmark8\t[remote rejected] (hook failure)
!\tdeadbeef:refs/heads/bookmark9\t[remote rejected]
Done";
    const SAMPLE_OK_STDERR: &[u8] = b"";

    #[test]
    fn test_parse_no_such_remote() {
        assert_eq!(
            parse_no_such_remote(SAMPLE_NO_SUCH_REPOSITORY_ERROR),
            Some("origin".to_string())
        );
        assert_eq!(
            parse_no_such_remote(SAMPLE_NO_SUCH_REMOTE_ERROR),
            Some("origin".to_string())
        );
        assert_eq!(parse_no_such_remote(SAMPLE_NO_REMOTE_REF_ERROR), None);
        assert_eq!(
            parse_no_such_remote(SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR),
            None
        );
        assert_eq!(
            parse_no_such_remote(SAMPLE_PUSH_REFS_PORCELAIN_OUTPUT),
            None
        );
        assert_eq!(parse_no_such_remote(SAMPLE_OK_STDERR), None);
    }

    #[test]
    fn test_parse_no_remote_ref() {
        assert_eq!(parse_no_remote_ref(SAMPLE_NO_SUCH_REPOSITORY_ERROR), None);
        assert_eq!(parse_no_remote_ref(SAMPLE_NO_SUCH_REMOTE_ERROR), None);
        assert_eq!(
            parse_no_remote_ref(SAMPLE_NO_REMOTE_REF_ERROR),
            Some("refs/heads/noexist".to_string())
        );
        assert_eq!(
            parse_no_remote_ref(SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR),
            None
        );
        assert_eq!(parse_no_remote_ref(SAMPLE_PUSH_REFS_PORCELAIN_OUTPUT), None);
        assert_eq!(parse_no_remote_ref(SAMPLE_OK_STDERR), None);
    }

    #[test]
    fn test_parse_no_remote_tracking_branch() {
        assert_eq!(
            parse_no_remote_tracking_branch(SAMPLE_NO_SUCH_REPOSITORY_ERROR),
            None
        );
        assert_eq!(
            parse_no_remote_tracking_branch(SAMPLE_NO_SUCH_REMOTE_ERROR),
            None
        );
        assert_eq!(
            parse_no_remote_tracking_branch(SAMPLE_NO_REMOTE_REF_ERROR),
            None
        );
        assert_eq!(
            parse_no_remote_tracking_branch(SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR),
            Some("bookmark".to_string())
        );
        assert_eq!(
            parse_no_remote_tracking_branch(SAMPLE_PUSH_REFS_PORCELAIN_OUTPUT),
            None
        );
        assert_eq!(parse_no_remote_tracking_branch(SAMPLE_OK_STDERR), None);
    }

    #[test]
    fn test_parse_ref_pushes() {
        assert!(parse_ref_pushes(SAMPLE_NO_SUCH_REPOSITORY_ERROR).is_err());
        assert!(parse_ref_pushes(SAMPLE_NO_SUCH_REMOTE_ERROR).is_err());
        assert!(parse_ref_pushes(SAMPLE_NO_REMOTE_REF_ERROR).is_err());
        assert!(parse_ref_pushes(SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR).is_err());
        let GitPushStats {
            pushed,
            rejected,
            remote_rejected,
        } = parse_ref_pushes(SAMPLE_PUSH_REFS_PORCELAIN_OUTPUT).unwrap();
        assert_eq!(
            pushed,
            [
                "refs/heads/bookmark1",
                "refs/heads/bookmark2",
                "refs/heads/bookmark3",
                "refs/heads/bookmark4",
                "refs/heads/bookmark5",
            ]
            .map(GitRefNameBuf::from)
        );
        assert_eq!(
            rejected,
            vec![
                (
                    "refs/heads/bookmark6".into(),
                    Some("failure lease".to_string())
                ),
                ("refs/heads/bookmark7".into(), None),
            ]
        );
        assert_eq!(
            remote_rejected,
            vec![
                (
                    "refs/heads/bookmark8".into(),
                    Some("hook failure".to_string())
                ),
                ("refs/heads/bookmark9".into(), None)
            ]
        );
        assert!(parse_ref_pushes(SAMPLE_OK_STDERR).is_err());
    }

    #[test]
    fn test_read_to_end_with_progress() {
        let read = |sample: &[u8]| {
            let mut progress = Vec::new();
            let mut sideband = Vec::new();
            let mut callbacks = RemoteCallbacks::default();
            let mut progress_cb = |p: &Progress| progress.push(p.clone());
            callbacks.progress = Some(&mut progress_cb);
            let mut sideband_cb = |s: &[u8]| sideband.push(s.to_owned());
            callbacks.sideband_progress = Some(&mut sideband_cb);
            let output = read_to_end_with_progress(&mut &sample[..], &mut callbacks).unwrap();
            (output, sideband, progress)
        };
        const DUMB_SUFFIX: &str = "        ";
        let sample = formatdoc! {"
            remote: line1{DUMB_SUFFIX}
            blah blah
            remote: line2.0{DUMB_SUFFIX}\rremote: line2.1{DUMB_SUFFIX}
            remote: line3{DUMB_SUFFIX}
            Resolving deltas: (12/24)
            some error message
        "};

        let (output, sideband, progress) = read(sample.as_bytes());
        assert_eq!(
            sideband,
            [
                "line1", "\n", "line2.0", "\r", "line2.1", "\n", "line3", "\n"
            ]
            .map(|s| s.as_bytes().to_owned())
        );
        assert_eq!(output, b"blah blah\nsome error message\n");
        insta::assert_debug_snapshot!(progress, @r"
        [
            Progress {
                bytes_downloaded: None,
                overall: 0.5,
            },
        ]
        ");

        // without last newline
        let (output, sideband, _progress) = read(sample.as_bytes().trim_end());
        assert_eq!(
            sideband,
            [
                "line1", "\n", "line2.0", "\r", "line2.1", "\n", "line3", "\n"
            ]
            .map(|s| s.as_bytes().to_owned())
        );
        assert_eq!(output, b"blah blah\nsome error message");
    }

    #[test]
    fn test_read_progress_line() {
        assert_eq!(
            read_progress_line(b"Receiving objects: (42/100)\r"),
            Some((42, 100))
        );
        assert_eq!(
            read_progress_line(b"Resolving deltas: (0/1000)\r"),
            Some((0, 1000))
        );
        assert_eq!(read_progress_line(b"Receiving objects: (420/100)\r"), None);
        assert_eq!(
            read_progress_line(b"remote: this is something else\n"),
            None
        );
        assert_eq!(read_progress_line(b"fatal: this is a git error\n"), None);
    }

    #[test]
    fn test_parse_unknown_option() {
        assert_eq!(
            parse_unknown_option(b"unknown option: --abc").unwrap(),
            "abc".to_string()
        );
        assert_eq!(
            parse_unknown_option(b"error: unknown option `abc'").unwrap(),
            "abc".to_string()
        );
        assert!(parse_unknown_option(b"error: unknown option: 'abc'").is_none());
    }

    #[test]
    fn test_initial_overall_progress_is_zero() {
        assert_eq!(GitProgress::default().to_progress().overall, 0.0);
    }
}
