use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use bstr::ByteSlice;
use thiserror::Error;

use crate::git::GitFetchError;
use crate::git::GitPushError;

#[derive(Error, Debug)]
pub enum GitSubprocessError {
    #[error("Could not find '{0}' executable in the OS path")]
    GitCommandNotFoundInPath(PathBuf),
    #[error("Could not execute git executable at path '{0}'")]
    GitCommandNotFound(PathBuf),
    #[error("Failed to execute the git process")]
    Spawn(std::io::Error),
    #[error("Failed to wait for the git process")]
    Wait(std::io::Error),
    #[error("Git process failed: {0}")]
    External(String),
}

#[derive(Error, Debug)]
pub enum GitError {
    #[error("No git remote named '{0}'")]
    NoSuchRemote(String),
    #[error(transparent)]
    Subprocess(#[from] GitSubprocessError),
}

impl From<GitError> for GitFetchError {
    fn from(value: GitError) -> Self {
        match value {
            GitError::NoSuchRemote(remote) => GitFetchError::NoSuchRemote(remote),
            GitError::Subprocess(error) => GitFetchError::Subprocess(error),
        }
    }
}

impl From<GitError> for GitPushError {
    fn from(value: GitError) -> Self {
        match value {
            GitError::NoSuchRemote(remote) => GitPushError::NoSuchRemote(remote),
            GitError::Subprocess(error) => GitPushError::Subprocess(error),
        }
    }
}

/// Context for creating git subprocesses
pub(crate) struct GitSubprocessContext<'a> {
    git_dir: PathBuf,
    git_path: &'a Path,
}

impl<'a> GitSubprocessContext<'a> {
    pub(crate) fn new(git_dir: impl Into<PathBuf>, git_path: &'a Path) -> Self {
        GitSubprocessContext {
            git_dir: git_dir.into(),
            git_path,
        }
    }

    pub(crate) fn from_git2(
        git_repo: &git2::Repository,
        git_path: &'a Path,
    ) -> Result<Self, GitError> {
        let git_dir = git_repo.path();

        Ok(Self::new(git_dir, git_path))
    }

    /// Spawn a git process
    fn spawn<I, S>(&self, args: I, stdout: Stdio) -> Result<Output, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut git_cmd = Command::new(self.git_path);
        // we cd into the work tree to avoid
        git_cmd
            .args([
                OsStr::new("--git-dir"),
                self.git_dir.as_ref(),
                OsStr::new("--bare"),
            ])
            .args(args)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped());
        tracing::debug!(cmd = ?git_cmd, "spawning a git subprocess");
        let child_git = git_cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                if self.git_path.is_absolute() {
                    GitSubprocessError::GitCommandNotFound(self.git_path.to_path_buf())
                } else {
                    GitSubprocessError::GitCommandNotFoundInPath(self.git_path.to_path_buf())
                }
            } else {
                GitSubprocessError::Spawn(e)
            }
        })?;

        let output = child_git
            .wait_with_output()
            .map_err(GitSubprocessError::Wait)?;
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
        let args = {
            let mut a = vec!["fetch".to_string(), "--prune".to_string()];
            if let Some(d) = depth {
                a.push(format!("--depth={d}"));
            }
            a.push("--".to_string());
            a.push(remote_name.to_string());
            a.push(refspec.to_string());
            a
        };

        let output = self.spawn(&args, Stdio::inherit())?;
        // we name the type to make sure that it is not meant to be used
        let _: () = parse_git_fetch_output(output, prunes)?;

        Ok(())
    }

    /// How we retrieve the remote's default branch:
    ///
    /// `git remote show <remote_name>`
    ///
    /// dumps a lot of information about the remote, with a line such as:
    /// `  HEAD branch: <default_branch>`
    pub(crate) fn spawn_remote_show(&self, remote_name: &str) -> Result<Option<String>, GitError> {
        let output = self.spawn(["remote", "show", "--", remote_name], Stdio::piped())?;

        let output = parse_git_remote_show_output(output)?;

        // find the HEAD branch line in the output
        let branch_name = String::from_utf8(output.stdout)
            .map_err(|e| {
                GitSubprocessError::External(format!("git remote output is not utf-8: {e:?}"))
            })
            .map_err(GitError::Subprocess)?
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

    /// Prune particular branches
    ///
    /// Even if git fetch has --prune, if a branch is not found it will not be
    /// pruned on fetch
    pub(crate) fn spawn_branch_prune(
        &self,
        remote_name: &str,
        branch_name: &str,
    ) -> Result<(), GitError> {
        let refname = format!("{remote_name}/{branch_name}");
        let output = self.spawn(
            ["branch", "--remotes", "--delete", "--", &refname],
            Stdio::null(),
        )?;

        // we name the type to make sure that it is not meant to be used
        let _: () = parse_git_branch_prune_output(output)?;

        Ok(())
    }

    /// Push references to git
    ///
    /// All pushes are forced, using --force-with-lease to perform a test&set
    /// operation on the remote repository
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
        // it becomes a regular forced push
        let full_refspec = full_refspec.strip_prefix("+").unwrap_or(full_refspec);
        let args = {
            let mut a = vec!["push".to_string()];
            if let Some(refname) = remote_ref {
                a.push(format!(
                    "--force-with-lease={refname}:{}",
                    expected_remote_location.unwrap_or("")
                ));
            }

            a.extend_from_slice(&[
                "--".to_string(),
                remote_name.to_string(),
                full_refspec.to_string(),
            ]);
            a
        };

        let output = self.spawn(&args, Stdio::inherit())?;

        // parse git push output returns true if the test and set operation failed
        // because a reference moved on the remote
        let test_and_set_failed = parse_git_push_output(output)?;

        if test_and_set_failed {
            if let Some(dst_reference) = remote_ref {
                failed_ref_matches.push(dst_reference.to_string());
            }
        }

        Ok(())
    }
}

/// Generate a GitError::ExternalGitError if the stderr output was not
/// recognizable
fn external_git_error(stderr: &[u8]) -> GitError {
    GitError::Subprocess(GitSubprocessError::External(format!(
        "External git program failed:\n{}",
        stderr.to_str_lossy()
    )))
}

/// Parse no such remote errors output from git
///
/// To say this, git prints out a lot of things, but the first line is of the
/// form:
/// `fatal: '<branch_name>' does not appear to be a git repository`
/// or
/// `fatal: '<branch_name>': Could not resolve host: invalid-remote
fn parse_no_such_remote(stderr: &[u8]) -> Result<(), String> {
    if let Some(first_line) = stderr.lines().next() {
        if (first_line.starts_with_str("fatal: '")
            && first_line.ends_with_str("' does not appear to be a git repository"))
            || (first_line.starts_with_str("fatal: unable to access '")
                && first_line.ends_with_str("': Could not resolve host: invalid-remote"))
        {
            let mut split = first_line.split_str("\'");
            split.next(); // ignore prefix
            let branch_name = split.next();
            if let Some(bname) = branch_name {
                return Err(bname.to_str_lossy().to_string());
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
fn parse_no_remote_ref(stderr: &[u8]) -> Option<String> {
    if let Some(first_line) = stderr.lines().next() {
        if first_line.starts_with_str("fatal: couldn't find remote ref refs/heads/") {
            let mut sp = first_line.split_str("refs/heads/");
            sp.next();
            return Some(sp.next().unwrap().to_str_lossy().to_string());
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
fn parse_no_remote_tracking_branch(stderr: &[u8]) -> bool {
    if let Some(first_line) = stderr.lines().next() {
        return first_line.starts_with_str("error: remote-tracking branch")
            && (first_line.ends_with_str("not found") || first_line.ends_with_str("not found."));
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
fn parse_force_with_lease(stderr: &[u8]) -> bool {
    stderr
        .lines()
        .last()
        .map(|x| x.starts_with_str("error: failed to push some refs to "))
        .unwrap_or(false)
}

fn parse_git_fetch_output(output: Output, prunes: &mut Vec<String>) -> Result<(), GitError> {
    if output.status.success() {
        return Ok(());
    }

    // There are some git errors we want to parse out
    parse_no_such_remote(&output.stderr).map_err(GitError::NoSuchRemote)?;

    if let Some(branch_name) = parse_no_remote_ref(&output.stderr) {
        prunes.push(branch_name);
        return Ok(());
    }

    if parse_no_remote_tracking_branch(&output.stderr) {
        return Ok(());
    }

    Err(external_git_error(&output.stderr))
}

fn parse_git_remote_show_output(output: Output) -> Result<Output, GitError> {
    if output.status.success() {
        return Ok(output);
    }

    // There are some git errors we want to parse out
    parse_no_such_remote(&output.stderr).map_err(GitError::NoSuchRemote)?;

    Err(external_git_error(&output.stderr))
}

fn parse_git_branch_prune_output(output: Output) -> Result<(), GitError> {
    if output.status.success() {
        return Ok(());
    }

    // There are some git errors we want to parse out
    if parse_no_remote_tracking_branch(&output.stderr) {
        return Ok(());
    }

    Err(external_git_error(&output.stderr))
}

// true indicates force_with_lease failed due to mismatch
// with expected and actual reference locations on the remote
fn parse_git_push_output(output: Output) -> Result<bool, GitError> {
    if output.status.success() {
        return Ok(false);
    }

    parse_no_such_remote(&output.stderr).map_err(GitError::NoSuchRemote)?;
    if parse_force_with_lease(&output.stderr) {
        return Ok(true);
    }

    Err(external_git_error(&output.stderr))
}

#[cfg(test)]
mod test {
    use super::*;
    const SAMPLE_NO_SUCH_REMOTE_ERROR: &[u8] =
        r###"fatal: 'origin' does not appear to be a git repository
fatal: Could not read from remote repository.

Please make sure you have the correct access rights
and the repository exists. "###
            .as_bytes();
    const SAMPLE_NO_REMOTE_REF_ERROR: &[u8] =
        "fatal: couldn't find remote ref refs/heads/noexist".as_bytes();
    const SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR: &[u8] =
        "error: remote-tracking branch 'bookmark' not found".as_bytes();
    const SAMPLE_FORCE_WITH_LEASE_ERROR: &[u8] = r###"To origin
 ! [rejected]        cb17dcdc74d5c974e836ad8ee9e9beace9864e28 -> bookmark1 (stale info)
error: failed to push some refs to 'origin'"###
        .as_bytes();
    const SAMPLE_OK_STDERR: &[u8] = "".as_bytes();

    #[test]
    fn test_parse_no_such_remote() {
        assert_eq!(
            parse_no_such_remote(SAMPLE_NO_SUCH_REMOTE_ERROR),
            Err("origin".to_string())
        );
        assert_eq!(parse_no_such_remote(SAMPLE_NO_REMOTE_REF_ERROR), Ok(()));
        assert_eq!(
            parse_no_such_remote(SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR),
            Ok(())
        );
        assert_eq!(parse_no_such_remote(SAMPLE_FORCE_WITH_LEASE_ERROR), Ok(()));
        assert_eq!(parse_no_such_remote(SAMPLE_OK_STDERR), Ok(()));
    }

    #[test]
    fn test_parse_no_remote_ref() {
        assert_eq!(parse_no_remote_ref(SAMPLE_NO_SUCH_REMOTE_ERROR), None);
        assert_eq!(
            parse_no_remote_ref(SAMPLE_NO_REMOTE_REF_ERROR),
            Some("noexist".to_string())
        );
        assert_eq!(
            parse_no_remote_ref(SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR),
            None
        );
        assert_eq!(parse_no_remote_ref(SAMPLE_FORCE_WITH_LEASE_ERROR), None);
        assert_eq!(parse_no_remote_ref(SAMPLE_OK_STDERR), None);
    }

    #[test]
    fn test_parse_no_remote_tracking_branch() {
        assert!(!parse_no_remote_tracking_branch(
            SAMPLE_NO_SUCH_REMOTE_ERROR
        ));
        assert!(!parse_no_remote_tracking_branch(SAMPLE_NO_REMOTE_REF_ERROR));
        assert!(parse_no_remote_tracking_branch(
            SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR
        ));
        assert!(!parse_no_remote_tracking_branch(
            SAMPLE_FORCE_WITH_LEASE_ERROR
        ));
        assert!(!parse_no_remote_tracking_branch(SAMPLE_OK_STDERR));
    }

    #[test]
    fn test_parse_force_with_lease() {
        assert!(!parse_force_with_lease(SAMPLE_NO_SUCH_REMOTE_ERROR));
        assert!(!parse_force_with_lease(SAMPLE_NO_REMOTE_REF_ERROR));
        assert!(!parse_force_with_lease(
            SAMPLE_NO_REMOTE_TRACKING_BRANCH_ERROR
        ));
        assert!(parse_force_with_lease(SAMPLE_FORCE_WITH_LEASE_ERROR));
        assert!(!parse_force_with_lease(SAMPLE_OK_STDERR));
    }
}
