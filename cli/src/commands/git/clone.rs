// Copyright 2020-2023 The Jujutsu Authors
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

use std::fs;
use std::io;
use std::io::Write;
use std::mem;
use std::num::NonZeroU32;
use std::path::Path;

use jj_lib::git;
use jj_lib::git::GitFetchError;
use jj_lib::git::GitFetchStats;
use jj_lib::repo::Repo;
use jj_lib::str_util::StringPattern;
use jj_lib::workspace::Workspace;

use super::write_repository_level_trunk_alias;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::cli_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::command_error::CommandError;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::get_git_repo;
use crate::git_util::map_git_error;
use crate::git_util::print_git_import_stats;
use crate::git_util::with_remote_git_callbacks;
use crate::ui::Ui;

/// Create a new repo backed by a clone of a Git repo
///
/// The Git repo will be a bare git repo stored inside the `.jj/` directory.
#[derive(clap::Args, Clone, Debug)]
pub struct GitCloneArgs {
    /// URL or path of the Git repo to clone
    #[arg(value_hint = clap::ValueHint::DirPath)]
    source: String,
    /// Specifies the target directory for the Jujutsu repository clone.
    /// If not provided, defaults to a directory named after the last component
    /// of the source URL. The full directory path will be created if it
    /// doesn't exist.
    #[arg(value_hint = clap::ValueHint::DirPath)]
    destination: Option<String>,
    /// Name of the newly created remote
    #[arg(long = "remote", default_value = "origin")]
    remote_name: String,
    /// Whether or not to colocate the Jujutsu repo with the git repo
    #[arg(long)]
    colocate: bool,
    /// Create a shallow clone of the given depth
    #[arg(long)]
    depth: Option<NonZeroU32>,
}

fn absolute_git_source(cwd: &Path, source: &str) -> Result<String, CommandError> {
    // Git appears to turn URL-like source to absolute path if local git directory
    // exits, and fails because '$PWD/https' is unsupported protocol. Since it would
    // be tedious to copy the exact git (or libgit2) behavior, we simply let gix
    // parse the input as URL, rcp-like, or local path.
    let mut url = gix::url::parse(source.as_ref()).map_err(cli_error)?;
    url.canonicalize(cwd).map_err(user_error)?;
    // As of gix 0.68.0, the canonicalized path uses platform-native directory
    // separator, which isn't compatible with libgit2 on Windows.
    if url.scheme == gix::url::Scheme::File {
        url.path = gix::path::to_unix_separators_on_windows(mem::take(&mut url.path)).into_owned();
    }
    // It's less likely that cwd isn't utf-8, so just fall back to original source.
    Ok(String::from_utf8(url.to_bstring().into()).unwrap_or_else(|_| source.to_owned()))
}

fn clone_destination_for_source(source: &str) -> Option<&str> {
    let destination = source.strip_suffix(".git").unwrap_or(source);
    let destination = destination.strip_suffix('/').unwrap_or(destination);
    destination
        .rsplit_once(&['/', '\\', ':'][..])
        .map(|(_, name)| name)
}

fn is_empty_dir(path: &Path) -> bool {
    if let Ok(mut entries) = path.read_dir() {
        entries.next().is_none()
    } else {
        false
    }
}

pub fn cmd_git_clone(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitCloneArgs,
) -> Result<(), CommandError> {
    let remote_name = &args.remote_name;
    if command.global_args().at_operation.is_some() {
        return Err(cli_error("--at-op is not respected"));
    }
    let source = absolute_git_source(command.cwd(), &args.source)?;
    let wc_path_str = args
        .destination
        .as_deref()
        .or_else(|| clone_destination_for_source(&source))
        .ok_or_else(|| user_error("No destination specified and wasn't able to guess it"))?;
    let wc_path = command.cwd().join(wc_path_str);

    let wc_path_existed = wc_path.exists();
    if wc_path_existed && !is_empty_dir(&wc_path) {
        return Err(user_error(
            "Destination path exists and is not an empty directory",
        ));
    }

    // will create a tree dir in case if was deleted after last check
    fs::create_dir_all(&wc_path)
        .map_err(|err| user_error_with_message(format!("Failed to create {wc_path_str}"), err))?;

    // Canonicalize because fs::remove_dir_all() doesn't seem to like e.g.
    // `/some/path/.`
    let canonical_wc_path = dunce::canonicalize(&wc_path)
        .map_err(|err| user_error_with_message(format!("Failed to create {wc_path_str}"), err))?;
    let clone_result = do_git_clone(
        ui,
        command,
        args.colocate,
        args.depth,
        remote_name,
        &source,
        &canonical_wc_path,
    );
    if clone_result.is_err() {
        let clean_up_dirs = || -> io::Result<()> {
            fs::remove_dir_all(canonical_wc_path.join(".jj"))?;
            if args.colocate {
                fs::remove_dir_all(canonical_wc_path.join(".git"))?;
            }
            if !wc_path_existed {
                fs::remove_dir(&canonical_wc_path)?;
            }
            Ok(())
        };
        if let Err(err) = clean_up_dirs() {
            writeln!(
                ui.warning_default(),
                "Failed to clean up {}: {}",
                canonical_wc_path.display(),
                err
            )
            .ok();
        }
    }

    let (mut workspace_command, stats) = clone_result?;
    if let Some(default_branch) = &stats.default_branch {
        write_repository_level_trunk_alias(
            ui,
            workspace_command.repo_path(),
            remote_name,
            default_branch,
        )?;

        let default_branch_remote_ref = workspace_command
            .repo()
            .view()
            .get_remote_bookmark(default_branch, remote_name);
        if let Some(commit_id) = default_branch_remote_ref.target.as_normal().cloned() {
            let mut checkout_tx = workspace_command.start_transaction();
            // For convenience, create local bookmark as Git would do.
            checkout_tx
                .repo_mut()
                .track_remote_bookmark(default_branch, remote_name);
            if let Ok(commit) = checkout_tx.repo().store().get_commit(&commit_id) {
                checkout_tx.check_out(&commit)?;
            }
            checkout_tx.finish(ui, "check out git remote's default branch")?;
        }
    }
    Ok(())
}

fn do_git_clone(
    ui: &mut Ui,
    command: &CommandHelper,
    colocate: bool,
    depth: Option<NonZeroU32>,
    remote_name: &str,
    source: &str,
    wc_path: &Path,
) -> Result<(WorkspaceCommandHelper, GitFetchStats), CommandError> {
    let settings = command.settings_for_new_workspace(wc_path)?;
    let (workspace, repo) = if colocate {
        Workspace::init_colocated_git(&settings, wc_path)?
    } else {
        Workspace::init_internal_git(&settings, wc_path)?
    };
    let git_repo = get_git_repo(repo.store())?;
    writeln!(
        ui.status(),
        r#"Fetching into new repo in "{}""#,
        wc_path.display()
    )?;
    let mut workspace_command = command.for_workable_repo(ui, workspace, repo)?;
    maybe_add_gitignore(&workspace_command)?;
    git_repo.remote(remote_name, source).unwrap();
    let git_settings = workspace_command.settings().git_settings()?;
    let mut fetch_tx = workspace_command.start_transaction();

    let stats = with_remote_git_callbacks(ui, None, |cb| {
        git::fetch(
            fetch_tx.repo_mut(),
            &git_repo,
            remote_name,
            &[StringPattern::everything()],
            cb,
            &git_settings,
            depth,
        )
    })
    .map_err(|err| match err {
        GitFetchError::NoSuchRemote(_) => {
            panic!("shouldn't happen as we just created the git remote")
        }
        GitFetchError::GitImportError(err) => CommandError::from(err),
        GitFetchError::InternalGitError(err) => map_git_error(err),
        GitFetchError::InvalidBranchPattern => {
            unreachable!("we didn't provide any globs")
        }
    })?;
    print_git_import_stats(ui, fetch_tx.repo(), &stats.import_stats, true)?;
    fetch_tx.finish(ui, "fetch from git remote into empty repo")?;
    Ok((workspace_command, stats))
}

#[cfg(test)]
mod tests {
    use std::path::MAIN_SEPARATOR;

    use super::*;

    #[test]
    fn test_absolute_git_source() {
        // gix::Url::canonicalize() works even if the path doesn't exist.
        // However, we need to ensure that no symlinks exist at the test paths.
        let temp_dir = testutils::new_temp_dir();
        let cwd = dunce::canonicalize(temp_dir.path()).unwrap();
        let cwd_slash = cwd.to_str().unwrap().replace(MAIN_SEPARATOR, "/");

        // Local path
        assert_eq!(
            absolute_git_source(&cwd, "foo").unwrap(),
            format!("{cwd_slash}/foo")
        );
        assert_eq!(
            absolute_git_source(&cwd, r"foo\bar").unwrap(),
            if cfg!(windows) {
                format!("{cwd_slash}/foo/bar")
            } else {
                format!(r"{cwd_slash}/foo\bar")
            }
        );
        assert_eq!(
            absolute_git_source(&cwd.join("bar"), &format!("{cwd_slash}/foo")).unwrap(),
            format!("{cwd_slash}/foo")
        );

        // rcp-like
        assert_eq!(
            absolute_git_source(&cwd, "git@example.org:foo/bar.git").unwrap(),
            "git@example.org:foo/bar.git"
        );
        // URL
        assert_eq!(
            absolute_git_source(&cwd, "https://example.org/foo.git").unwrap(),
            "https://example.org/foo.git"
        );
        // Custom scheme isn't an error
        assert_eq!(
            absolute_git_source(&cwd, "custom://example.org/foo.git").unwrap(),
            "custom://example.org/foo.git"
        );
        // Password shouldn't be redacted (gix::Url::to_string() would do)
        assert_eq!(
            absolute_git_source(&cwd, "https://user:pass@example.org/").unwrap(),
            "https://user:pass@example.org/"
        );
    }
}
