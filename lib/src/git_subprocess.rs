use std::ffi::OsStr;
use std::fmt;
use std::num::NonZeroU32;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use thiserror::Error;

use crate::git::GitFetchError;
use crate::git::GitPushError;

/// New-type for comparable io errors
pub struct IoError(std::io::Error);
impl PartialEq for IoError {
    fn eq(&self, other: &Self) -> bool {
        self.0.kind().eq(&other.0.kind())
    }
}
impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}
impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::fmt::Debug for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Error, Debug, PartialEq)]
pub enum GitError {
    #[error("No git remote named '{0}'")]
    NoSuchRemote(String),
    #[error("`git` executable not found")]
    GitNotFound,
    #[error("Failed to fork git process")]
    GitForkError(#[from] IoError),
    #[error("git process failed: {0}")]
    GitExternalError(String),
    #[error("failed to convert path {0:?} to a string")]
    PathConversionError(std::path::PathBuf),
}
impl From<std::io::Error> for GitError {
    fn from(value: std::io::Error) -> Self {
        GitError::GitForkError(IoError(value))
    }
}

impl From<GitError> for GitFetchError {
    fn from(value: GitError) -> Self {
        match value {
            GitError::NoSuchRemote(remote) => GitFetchError::NoSuchRemote(remote),
            GitError::GitNotFound => GitFetchError::GitNotFound,
            GitError::GitForkError(io_err) => GitFetchError::GitForkError(io_err.0),
            GitError::GitExternalError(error) => GitFetchError::GitExternalError(error),
            GitError::PathConversionError(path) => GitFetchError::PathConversionError(path),
        }
    }
}

impl From<GitError> for GitPushError {
    fn from(value: GitError) -> Self {
        match value {
            GitError::NoSuchRemote(remote) => GitPushError::NoSuchRemote(remote),
            GitError::GitNotFound => GitPushError::GitNotFound,
            GitError::GitForkError(io_err) => GitPushError::GitForkError(io_err),
            GitError::GitExternalError(error) => GitPushError::GitExternalError(error),
            GitError::PathConversionError(path) => GitPushError::PathConversionError(path),
        }
    }
}

/// Context for creating git subprocesses
pub(crate) struct GitSubprocessContext {
    git_dir: String,
    work_tree: String,
}

impl GitSubprocessContext {
    pub(crate) fn new(git_dir: impl ToString, work_tree: impl ToString) -> Self {
        GitSubprocessContext {
            git_dir: git_dir.to_string(),
            work_tree: work_tree.to_string(),
        }
    }

    pub(crate) fn from_git2(git_repo: &git2::Repository) -> Result<Self, GitError> {
        let git_dir = git_repo
            .path()
            .to_str()
            .ok_or_else(|| GitError::PathConversionError(git_repo.path().to_path_buf()))?;
        let work_tree = work_tree_from_git_dir(git_repo.path())
            .map(|p| {
                p.to_str()
                    .map(|x| x.to_string())
                    .ok_or(GitError::PathConversionError(p))
            })
            .map_err(GitError::GitExternalError)??;

        Ok(Self::new(git_dir, work_tree))
    }

    fn spawn(&self, args: &[String], stdout: Stdio) -> Result<Output, GitError> {
        let full_args = {
            let mut v = Vec::with_capacity(args.len() + 2);
            v.push(format!("--git-dir={}", self.git_dir));
            v.push(format!("--work-tree={}", self.work_tree));
            v.extend_from_slice(args);
            v
        };

        tracing::debug!("shelling out to `git {}`", args.join(" "));
        let remote_git = Command::new("git")
            .args(&full_args)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    GitError::GitNotFound
                } else {
                    e.into()
                }
            })?;

        let output = remote_git.wait_with_output()?;
        Ok(output)
    }

    /// Perform a git fetch
    pub(crate) fn spawn_fetch(
        &self,
        remote_name: &str,
        depth: Option<NonZeroU32>,
        refspec: &str,
        prunes: &mut Vec<String>,
    ) -> Result<(), GitError> {
        let depth_arg = depth.map(|x| format!("--depth={x}"));
        let args = {
            let mut a = vec!["fetch".to_string(), "--prune".to_string()];
            if let Some(depth) = depth_arg {
                a.push(depth);
            }
            a.push(remote_name.to_string());
            a.push(refspec.to_string());
            a
        };

        let output = self.spawn(&args, Stdio::inherit())?;
        parse_git_fetch_output(output, prunes)?;

        Ok(())
    }

    /// How we retrieve the remote's default branch:
    ///
    /// `git remote show <remote_name>`
    ///
    /// dumps a lot of information about the remote, with a line such as:
    /// `  HEAD branch: <default_branch>`
    pub(crate) fn spawn_remote_show(&self, remote_name: &str) -> Result<Option<String>, GitError> {
        let output = self.spawn(
            &[
                "remote".to_string(),
                "show".to_string(),
                remote_name.to_string(),
            ],
            Stdio::piped(),
        )?;

        let output = parse_git_remote_show_output(output)?;

        // find the HEAD branch line in the output
        let branch_name = String::from_utf8(output.stdout)
            .map_err(|e| {
                GitError::GitExternalError(format!("git remote output is not utf-8: {e:?}"))
            })?
            .lines()
            .map(|x| x.trim())
            .find(|x| x.starts_with("HEAD branch:"))
            .and_then(|x| x.split(" ").last().map(|y| y.trim().to_string()));

        // git will output (unknown) if there is no default branch. we want it to be a
        // none value
        if let Some(x) = branch_name.as_deref() {
            if x == "(unknown)" {
                return Ok(None);
            }
        }
        Ok(branch_name)
    }

    pub(crate) fn spawn_branch_prune(
        &self,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), GitError> {
        let output = self.spawn(
            &[
                "branch".to_string(),
                "--remotes".to_string(),
                "--delete".to_string(),
                format!("{remote_name}/{branch_name}"),
            ],
            Stdio::null(),
        )?;

        parse_git_branch_prune_output(output)?;

        Ok(())
    }

    pub(crate) fn spawn_push(
        &self,
        remote_name: &str,
        remote_ref: Option<&str>,
        full_refspec: &str,
        expected_remote_location: Option<&str>,
        failed_ref_matches: &mut Vec<String>,
    ) -> Result<(), GitError> {
        // we use --force-with-lease, so we are already force pushing
        // (which is the behaviour a leading + signifies)
        //
        // if we leave the leading +, the test and set operation is ignored and
        // it becomes a regular push
        let full_refspec = remove_leading_plus(full_refspec);
        let args = {
            let mut a = vec!["push".to_string()];
            if let Some(refname) = remote_ref {
                a.push(format!(
                    "--force-with-lease={refname}:{}",
                    expected_remote_location.unwrap_or("")
                ));
            }

            a.extend_from_slice(&[remote_name.to_string(), full_refspec.to_string()]);
            a
        };

        let output = self.spawn(&args, Stdio::inherit())?;

        // parse git push output returns true if the test and set operation failed
        // because a reference moved on the remote
        if parse_git_push_output(output)? {
            if let Some(dst_reference) = remote_ref {
                failed_ref_matches.push(dst_reference.to_string());
            }
        }

        Ok(())
    }
}

fn remove_leading_plus(refspec: &str) -> &str {
    if refspec.starts_with("+") {
        let mut chars = refspec.chars();
        chars.next();
        chars.as_str()
    } else {
        refspec
    }
}

/// Get the work tree dir from the git dir
///
/// There are two possible options:
///  - on a bare git repo, the dir has a parent named .jj that sits on the
///    workspace root
///  - on a colocated .git dir, it is already on the workspace root
fn work_tree_from_git_dir(git_dir: &Path) -> Result<PathBuf, String> {
    if git_dir.file_name() == Some(OsStr::new(".git")) {
        git_dir
            .parent()
            .map(|x| x.to_path_buf())
            .ok_or(format!("git repo had no parent: {}", git_dir.display()))
    } else if git_dir.file_name() == Some(OsStr::new("git")) {
        let mut it = git_dir
            .ancestors()
            .skip_while(|dir| !dir.ends_with(Path::new(".jj")));
        it.next().map(|x| x.to_path_buf()).ok_or(format!(
            "could not find .jj dir in git dir path: {}",
            git_dir.display()
        ))
    } else {
        Err(format!(
            "git dir is not named `git` nor `.git`: {}",
            git_dir.display()
        ))
    }
}

/// Parse no such remote errors output from git
///
/// To say this, git prints out a lot of things, but the first line is of the
/// form:
/// `fatal: '<branch_name>' does not appear to be a git repository`
/// or
/// `fatal: '<branch_name>': Could not resolve host: invalid-remote
fn parse_no_such_remote(stderr: &str) -> Result<(), String> {
    if let Some(first_line) = stderr.lines().next() {
        if (first_line.starts_with("fatal: '")
            && first_line.ends_with("' does not appear to be a git repository"))
            || (first_line.starts_with("fatal: unable to access '")
                && first_line.ends_with("': Could not resolve host: invalid-remote"))
        {
            let mut split = first_line.split('\'');
            split.next(); // ignore prefix
            let branch_name = split.next();
            if let Some(bname) = branch_name {
                return Err(bname.to_string());
            }
        }
    }

    Ok(())
}

/// Parse error from refspec not present on the remote
///
/// This returns the branch to prune if it is found, None if this wasn't the
/// error
///
/// On git fetch even though --prune is specified, if a particular
/// refspec is asked for but not present in the remote, git will error out.
///
/// The first line is of the form:
/// `fatal: couldn't find remote ref refs/heads/<ref>`
fn parse_no_remote_ref(stderr: &str) -> Option<String> {
    if let Some(first_line) = stderr.lines().next() {
        if first_line.starts_with("fatal: couldn't find remote ref refs/heads/") {
            let mut sp = first_line.split("refs/heads/");
            sp.next();
            return Some(sp.next().unwrap().to_string());
        }
    }

    None
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
fn parse_no_remote_tracking_branch(stderr: &str) -> bool {
    if let Some(first_line) = stderr.lines().next() {
        return first_line.starts_with("error: remote-tracking branch")
            && (first_line.ends_with("not found") || first_line.ends_with("not found."));
    }

    false
}

/// Parse errors from --force-with-lease
///
/// Return true if there was an error
///
/// When a ref has moved on the remote, git responds with
/// `
/// To <remote_url>
///  ! [rejected]        <note> -> <branch_name> (stale info)
/// error: failed to push some refs to <remote_url>
/// `
///
/// Because we only push one branch at a time,
/// we just look at the last line to report
/// whether this was the problem or not.
/// It is up to the caller to know which ref it was trying to push
fn parse_force_with_lease_error(stderr: &str) -> bool {
    stderr
        .lines()
        .last()
        .map(|x| x.starts_with("error: failed to push some refs to "))
        .unwrap_or(false)
}

fn parse_git_fetch_output(output: Output, prunes: &mut Vec<String>) -> Result<(), GitError> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8(output.stderr.clone()).map_err(|e| {
        GitError::GitExternalError(format!(
            "external git program failed with non-utf8 output: {e:?}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })?;

    // There are some git errors we want to parse out
    parse_no_such_remote(&stderr).map_err(GitError::NoSuchRemote)?;

    if let Some(branch_name) = parse_no_remote_ref(&stderr) {
        prunes.push(branch_name);
        return Ok(());
    }

    if parse_no_remote_tracking_branch(&stderr) {
        return Ok(());
    }

    Err(GitError::GitExternalError(format!(
        "external git program failed:\n{stderr}",
    )))
}

fn parse_git_remote_show_output(output: Output) -> Result<Output, GitError> {
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8(output.stderr.clone()).map_err(|e| {
        GitError::GitExternalError(format!(
            "external git program failed with non-utf8 output: {e:?}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })?;

    // There are some git errors we want to parse out
    parse_no_such_remote(&stderr).map_err(GitError::NoSuchRemote)?;

    Err(GitError::GitExternalError(format!(
        "external git program failed:\n{stderr}",
    )))
}

fn parse_git_branch_prune_output(output: Output) -> Result<Output, GitError> {
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8(output.stderr.clone()).map_err(|e| {
        GitError::GitExternalError(format!(
            "external git program failed with non-utf8 output: {e:?}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })?;

    // There are some git errors we want to parse out
    if parse_no_remote_tracking_branch(&stderr) {
        return Ok(output);
    }

    Err(GitError::GitExternalError(format!(
        "external git program failed:\n{stderr}",
    )))
}

// true indicates force_with_lease failed due to mismatch
// with expected and actual reference locations on the remote
fn parse_git_push_output(output: Output) -> Result<bool, GitError> {
    if output.status.success() {
        return Ok(false);
    }

    let stderr = String::from_utf8(output.stderr.clone()).map_err(|e| {
        GitError::GitExternalError(format!(
            "external git program failed with non-utf8 output: {e:?}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })?;

    parse_no_such_remote(&stderr).map_err(GitError::NoSuchRemote)?;
    if parse_force_with_lease_error(&stderr) {
        return Ok(true);
    }

    Err(GitError::GitExternalError(format!(
        "external git program failed:\n{stderr}"
    )))
}
