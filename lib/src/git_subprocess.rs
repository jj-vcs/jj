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
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;

use bstr::ByteSlice as _;
use itertools::Itertools as _;
use thiserror::Error;

use crate::git::GitPushOptions;
use crate::git::GitPushStats;
use crate::git::GitSubprocessOptions;
use crate::git::RefToPush;
use crate::git_backend::GitBackend;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RemoteName;
use crate::subprocess_util::suppress_console_window;

// * 2.29.0 introduced `git fetch --no-write-fetch-head`
// * 2.40 still receives security patches (latest one was in Jan/2025)
// * 2.41.0 introduced `git fetch --porcelain`
// * 2.42.0 introduced `git worktree add --orphan`
// If bumped, please update ../../docs/install-and-setup.md
const MINIMUM_GIT_VERSION: &str = "2.42.0";

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
        "Git does not recognize required option: {0} (note: Jujutsu requires git >= \
         {MINIMUM_GIT_VERSION})"
    )]
    UnsupportedGitOption(String),
    #[error("Git process failed: {0}")]
    External(String),
}

/// Context for creating Git subprocesses
pub(crate) struct GitSubprocessContext {
    git_dir: PathBuf,
    options: GitSubprocessOptions,
}

impl GitSubprocessContext {
    pub(crate) fn new(git_dir: impl Into<PathBuf>, options: GitSubprocessOptions) -> Self {
        Self {
            git_dir: git_dir.into(),
            options,
        }
    }

    pub(crate) fn from_git_backend(
        git_backend: &GitBackend,
        options: GitSubprocessOptions,
    ) -> Self {
        Self::new(git_backend.git_repo_path(), options)
    }

    /// Create the Git command
    fn create_command(&self) -> Command {
        let mut git_cmd = Command::new(&self.options.executable_path);
        suppress_console_window(&mut git_cmd);

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
            // Disable translation so we can parse the output. We don't set
            // LC_ALL=C because it would change the encoding. Also note that
            // "C.UTF-8" locale isn't always available.
            .env_remove("LC_ALL")
            .env_remove("LANGUAGE")
            .env("LC_MESSAGES", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::piped());

        git_cmd.envs(&self.options.environment);

        git_cmd
    }

    /// Spawn the git command
    fn spawn_cmd(&self, mut git_cmd: Command) -> Result<Child, GitSubprocessError> {
        tracing::debug!(cmd = ?git_cmd, "spawning a git subprocess");

        git_cmd.spawn().map_err(|error| {
            if self.options.executable_path.is_absolute() {
                GitSubprocessError::Spawn {
                    path: self.options.executable_path.clone(),
                    error,
                }
            } else {
                GitSubprocessError::SpawnInPath {
                    path: self.options.executable_path.clone(),
                    error,
                }
            }
        })
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
        let mut command = self.create_command();
        command.stdout(Stdio::piped());
        // Exclude all fetch mappings to prevent push from updating tracking
        // refs. An empty `-c remote.<name>.fetch=` entry doesn't disable the
        // configured refspecs.
        //
        // Git checks negative refspecs by reversing positive mappings, even
        // when querying by source. Add a dummy mapping so every source reaches
        // the exclusion check. For refs/heads/main -> refs/remotes/origin/main,
        // it tries to match refs/heads/main against refs/remotes/origin/* and
        // dummy refs/*, and translates "back" to refs/jj/dummy/heads/main,
        // which matches ^refs/*.
        //
        // https://github.com/git/git/blob/v2.42.0/remote.c#L891
        command
            .arg("-c")
            .arg(format!(
                "remote.{}.fetch=refs/jj/dummy/*:refs/*",
                remote_name.as_str()
            ))
            .arg("-c")
            .arg(format!("remote.{}.fetch=^refs/*", remote_name.as_str()));
        // Currently jj does not support commit hooks, so we prevent git from running
        // them
        //
        // https://github.com/jj-vcs/jj/issues/3577 and https://github.com/jj-vcs/jj/issues/405
        // offer more context
        command.args(["push", "--porcelain", "--no-verify"]);
        if callback.needs_progress() {
            command.arg("--progress");
        }
        command.args(
            options
                .remote_push_options
                .iter()
                .map(|option| format!("--push-option={option}")),
        );
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

        let output = wait_with_progress(self.spawn_cmd(command)?, callback)?;

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

const ERROR_PREFIXES: &[&[u8]] = &[
    // error_builtin() in usage.c
    b"error: ",
    // die_message_builtin() in usage.c
    b"fatal: ",
    // usage_builtin() in usage.c
    b"usage: ",
    // handle_option() in git.c
    b"unknown option: ",
];

/// Parse no such remote errors output from git
///
/// Returns the remote that wasn't found
///
/// To say this, git prints out a lot of things, but the first line is of the
/// form:
/// `fatal: '<remote>' does not appear to be a git repository`
/// or
/// `fatal: '<remote>': Could not resolve host: invalid-remote`
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

/// Waits for the process while emitting sideband data through the callback.
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
    callback: &mut dyn GitSubprocessCallback,
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
        let stderr = read_to_end_with_progress(&mut child_stderr, callback)?;
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

fn read_to_end_with_progress<R: Read>(
    src: R,
    callback: &mut dyn GitSubprocessCallback,
) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(src);
    let mut data = Vec::new();
    let mut progress = GitProgress::default();

    loop {
        // progress sent through sideband channel may be terminated by \r
        let start = data.len();
        read_until_cr_or_lf(&mut reader, &mut data)?;
        let line = &data[start..];
        if line.is_empty() {
            break;
        }

        // capture error messages which will be interpreted by caller
        if ERROR_PREFIXES.iter().any(|prefix| line.starts_with(prefix)) {
            reader.read_to_end(&mut data)?;
            break;
        }

        // io::Error coming from callback shouldn't be propagated as an error of
        // "read" operation. The error is suppressed for now.
        // TODO: maybe intercept "push" progress? (see builtin/pack-objects.c)
        if update_progress(line, &mut progress.objects, b"Receiving objects:")
            || update_progress(line, &mut progress.deltas, b"Resolving deltas:")
            || update_progress(
                line,
                &mut progress.counted_objects,
                b"remote: Counting objects:",
            )
            || update_progress(
                line,
                &mut progress.compressed_objects,
                b"remote: Compressing objects:",
            )
        {
            callback.progress(&progress).ok();
            data.truncate(start);
        } else if let Some(message) = line.strip_prefix(b"remote: ") {
            let (body, term) = trim_sideband_line(message);
            callback.remote_sideband(body, term).ok();
            data.truncate(start);
        } else {
            let (body, term) = trim_sideband_line(line);
            callback.local_sideband(body, term).ok();
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
            let mut callback = GitSubprocessCapture::default();
            let output = read_to_end_with_progress(&mut &sample[..], &mut callback).unwrap();
            (output, callback)
        };
        const DUMB_SUFFIX: &str = "        ";
        let sample = formatdoc! {"
            remote: line1{DUMB_SUFFIX}
            blah blah
            remote: line2.0{DUMB_SUFFIX}\rremote: line2.1{DUMB_SUFFIX}
            remote: line3{DUMB_SUFFIX}
            Resolving deltas: (12/24)
            fatal: some error message
            continues
        "};

        let (output, callback) = read(sample.as_bytes());
        assert_eq!(callback.local_sideband, ["blah blah", "\n"]);
        assert_eq!(
            callback.remote_sideband,
            [
                "line1", "\n", "line2.0", "\r", "line2.1", "\n", "line3", "\n"
            ]
        );
        assert_eq!(output, b"fatal: some error message\ncontinues\n");
        insta::assert_debug_snapshot!(callback.progress, @"
        [
            GitProgress {
                deltas: (
                    12,
                    24,
                ),
                objects: (
                    0,
                    0,
                ),
                counted_objects: (
                    0,
                    0,
                ),
                compressed_objects: (
                    0,
                    0,
                ),
            },
        ]
        ");

        // without last newline
        let (output, callback) = read(sample.as_bytes().trim_end());
        assert_eq!(
            callback.remote_sideband,
            [
                "line1", "\n", "line2.0", "\r", "line2.1", "\n", "line3", "\n"
            ]
        );
        assert_eq!(output, b"fatal: some error message\ncontinues");
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
        assert_eq!(GitProgress::default().overall(), 0.0);
    }
}
