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
use std::io::Write as _;
use std::num::NonZeroU32;
use std::path::Path;

use itertools::Itertools as _;
use jj_lib::file_util;
use jj_lib::git;
use jj_lib::git::FetchTagsOverride;
use jj_lib::git::GitFetch;
use jj_lib::git::GitSettings;
use jj_lib::git::expand_fetch_refspecs;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringExpression;
use jj_lib::workspace::Workspace;

use super::write_repository_level_trunk_alias;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::commands::git::FetchTagsMode;
use crate::commands::git::maybe_add_gitignore;
use crate::git_util::absolute_git_url;
use crate::git_util::load_git_import_options;
use crate::git_util::print_git_import_stats;
use crate::git_util::with_remote_git_callbacks;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Create a new repo backed by a clone of a Git repo
#[derive(clap::Args, Clone, Debug)]
pub struct GitCloneArgs {
    /// URL or path of the Git repo to clone
    ///
    /// Local path will be resolved to absolute form.
    #[arg(value_hint = clap::ValueHint::Url)]
    source: String,
    /// Specifies the target directory for the Jujutsu repository clone.
    /// If not provided, defaults to a directory named after the last component
    /// of the source URL. The full directory path will be created if it
    /// doesn't exist.
    #[arg(value_hint = clap::ValueHint::DirPath)]
    destination: Option<String>,
    /// Name of the newly created remote
    #[arg(long = "remote", default_value = "origin")]
    remote_name: RemoteNameBuf,
    /// Colocate the Jujutsu repo with the git repo
    ///
    /// Specifies that the `jj` repo should also be a valid `git` repo, allowing
    /// the use of both `jj` and `git` commands in the same directory.
    ///
    /// The repository will contain a `.git` dir in the top-level. Regular Git
    /// tools will be able to operate on the repo.
    ///
    /// **This is the default**, and this option has no effect, unless the
    /// [git.colocate config] is set to `false`.
    ///
    /// [git.colocate config]:
    ///     https://docs.jj-vcs.dev/latest/config/#default-colocation
    #[arg(long)]
    colocate: bool,
    /// Disable colocation of the Jujutsu repo with the git repo
    ///
    /// Prevent Git tools that are unaware of `jj` and regular Git commands from
    /// operating on the repo. The Git repository that stores most of the repo
    /// data will be hidden inside a sub-directory of the `.jj` directory.
    ///
    /// See [colocation docs] for some minor advantages of non-colocated
    /// workspaces.
    ///
    /// [colocation docs]:
    ///     https://docs.jj-vcs.dev/latest/git-compatibility/#colocated-jujutsugit-repos
    #[arg(long, conflicts_with = "colocate")]
    no_colocate: bool,
    /// Create a shallow clone of the given depth
    #[arg(long)]
    depth: Option<NonZeroU32>,
    /// Configure when to fetch tags
    ///
    /// Unless otherwise specified, the initial clone will fetch all tags,
    /// while all subsequent fetches will only fetch included tags.
    #[arg(long, value_enum)]
    fetch_tags: Option<FetchTagsMode>,
    /// Name of the branch to fetch and use as the parent of the working-copy
    /// change
    ///
    /// If not present, all branches are fetched and the repository's default
    /// branch is used as parent of the working-copy change.
    ///
    /// By default, the specified pattern matches branch names with glob syntax,
    /// but only `*` is expanded. Other wildcard characters such as `?` are
    /// *not* supported. Patterns can be repeated or combined with [logical
    /// operators] to specify multiple branches, but only union and negative
    /// intersection are supported. If there are multiple matching branches, the
    /// first exact branch name is used as the working-copy parent.
    ///
    /// Examples: `push-*`, `(push-* | foo/*) ~ foo/unwanted`
    ///
    /// [logical operators]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long, short, alias = "bookmark")]
    branch: Option<Vec<String>>,
}

fn clone_destination_for_source(source: &str) -> Option<&str> {
    let destination = source.strip_suffix(".git").unwrap_or(source);
    let destination = destination.strip_suffix('/').unwrap_or(destination);
    destination
        .rsplit_once(&['/', '\\', ':'][..])
        .map(|(_, name)| name)
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
    let source = absolute_git_url(command.cwd(), &args.source)?;
    let wc_path_str = args
        .destination
        .as_deref()
        .or_else(|| clone_destination_for_source(&source))
        .ok_or_else(|| user_error("No destination specified and wasn't able to guess it"))?;
    let wc_path = command.cwd().join(wc_path_str);

    let wc_path_existed = wc_path.exists();
    if wc_path_existed && !file_util::is_empty_dir(&wc_path)? {
        return Err(user_error(
            "Destination path exists and is not an empty directory",
        ));
    }

    // will create a tree dir in case if was deleted after last check
    fs::create_dir_all(&wc_path)
        .map_err(|err| user_error_with_message(format!("Failed to create {wc_path_str}"), err))?;

    let colocate = if command.settings().get_bool("git.colocate")? {
        !args.no_colocate
    } else {
        args.colocate
    };
    let bookmark_expr = match &args.branch {
        Some(texts) => parse_union_name_patterns(ui, texts)?,
        None => StringExpression::all(),
    };

    // Canonicalize because fs::remove_dir_all() doesn't seem to like e.g.
    // `/some/path/.`
    let canonical_wc_path = dunce::canonicalize(&wc_path)
        .map_err(|err| user_error_with_message(format!("Failed to create {wc_path_str}"), err))?;

    let clone_result = (|| -> Result<_, CommandError> {
        let workspace_command = init_workspace(ui, command, &canonical_wc_path, colocate)?;
        let mut workspace_command = configure_remote(
            ui,
            command,
            workspace_command,
            remote_name,
            &source,
            // If not explicitly specified on the CLI, configure the remote for only fetching
            // included tags for future fetches.
            args.fetch_tags.unwrap_or(FetchTagsMode::Included),
            &bookmark_expr,
        )?;
        let default_branch = fetch_new_remote(
            ui,
            &mut workspace_command,
            remote_name,
            // If we add default fetch patterns to jj's config, these patterns
            // will be loaded here?
            &bookmark_expr,
            args.depth,
            args.fetch_tags,
        )?;
        Ok((workspace_command, default_branch))
    })();
    if clone_result.is_err() {
        let clean_up_dirs = || -> io::Result<()> {
            let sub_dirs = [Some(".jj"), colocate.then_some(".git")];
            for &name in sub_dirs.iter().flatten() {
                let dir = canonical_wc_path.join(name);
                fs::remove_dir_all(&dir).or_else(|err| match err.kind() {
                    io::ErrorKind::NotFound => Ok(()),
                    _ => Err(err),
                })?;
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

    let (mut workspace_command, (working_branch, working_is_default)) = clone_result?;

    if let Some(name) = &working_branch {
        let working_symbol = name.to_remote_symbol(remote_name);
        if working_is_default {
            write_repository_level_trunk_alias(ui, workspace_command.repo_path(), working_symbol)?;
        }
        let working_branch_remote_ref = workspace_command
            .repo()
            .view()
            .get_remote_bookmark(working_symbol);
        if let Some(commit_id) = working_branch_remote_ref.target.as_normal().cloned() {
            let mut tx = workspace_command.start_transaction();
            if let Ok(commit) = tx.repo().store().get_commit(&commit_id) {
                tx.check_out(&commit)?;
            }
            tx.finish(
                ui,
                format!("check out git remote's branch: {}", name.as_symbol()),
            )?;
        }
    }

    if colocate {
        writeln!(
            ui.hint_default(),
            r"Running `git clean -xdf` will remove `.jj/`!",
        )?;
    }

    Ok(())
}

fn init_workspace(
    ui: &Ui,
    command: &CommandHelper,
    wc_path: &Path,
    colocate: bool,
) -> Result<WorkspaceCommandHelper, CommandError> {
    let settings = command.settings_for_new_workspace(wc_path)?;
    let (workspace, repo) = if colocate {
        Workspace::init_colocated_git(&settings, wc_path)?
    } else {
        Workspace::init_internal_git(&settings, wc_path)?
    };
    let workspace_command = command.for_workable_repo(ui, workspace, repo)?;
    maybe_add_gitignore(&workspace_command)?;
    Ok(workspace_command)
}

fn configure_remote(
    ui: &Ui,
    command: &CommandHelper,
    mut workspace_command: WorkspaceCommandHelper,
    remote_name: &RemoteName,
    source: &str,
    fetch_tags: FetchTagsMode,
    bookmark_expr: &StringExpression,
) -> Result<WorkspaceCommandHelper, CommandError> {
    let mut tx = workspace_command.start_transaction();
    git::add_remote(
        tx.repo_mut(),
        remote_name,
        source,
        None,
        fetch_tags.as_fetch_tags(),
        bookmark_expr,
    )?;
    tx.finish(ui, format!("add git remote {}", remote_name.as_symbol()))?;
    // Reload workspace to apply new remote configuration to
    // gix::ThreadSafeRepository behind the store.
    let workspace = command.load_workspace_at(
        workspace_command.workspace_root(),
        workspace_command.settings(),
    )?;
    let op = workspace
        .repo_loader()
        .load_operation(workspace_command.repo().op_id())?;
    let repo = workspace.repo_loader().load_at(&op)?;
    command.for_workable_repo(ui, workspace, repo)
}

fn fetch_new_remote(
    ui: &Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    remote_name: &RemoteName,
    bookmark_expr: &StringExpression,
    depth: Option<NonZeroU32>,
    fetch_tags: Option<FetchTagsMode>,
) -> Result<(Option<RefNameBuf>, bool), CommandError> {
    writeln!(
        ui.status(),
        r#"Fetching into new repo in "{}""#,
        workspace_command.workspace_root().display()
    )?;
    let settings = workspace_command.settings();
    let git_settings = GitSettings::from_settings(settings)?;
    let remote_settings = settings.remote_settings()?;
    let import_options = load_git_import_options(ui, &git_settings, &remote_settings)?;
    let should_track_default = settings.get_bool("git.track-default-bookmark-on-clone")?;
    let mut tx = workspace_command.start_transaction();
    let (default_branch, import_stats) = {
        let mut git_fetch = GitFetch::new(tx.repo_mut(), &git_settings, &import_options)?;

        let fetch_refspecs = expand_fetch_refspecs(remote_name, bookmark_expr.clone())?;

        with_remote_git_callbacks(ui, |cb| {
            git_fetch.fetch(
                remote_name,
                fetch_refspecs,
                cb,
                depth,
                match fetch_tags {
                    // If not explicitly specified on the CLI, override the remote
                    // configuration and fetch all tags by default since this is
                    // the Git default behavior.
                    None => Some(FetchTagsOverride::AllTags),

                    // Technically by this point the remote should already be
                    // configured based on the CLI parameters so we shouldn't *need*
                    // to apply an override here but all the cases are expanded here
                    // for clarity.
                    Some(FetchTagsMode::All) => Some(FetchTagsOverride::AllTags),
                    Some(FetchTagsMode::None) => Some(FetchTagsOverride::NoTags),
                    Some(FetchTagsMode::Included) => None,
                },
            )
        })?;

        let import_stats = git_fetch.import_refs()?;

        let default_branch = git_fetch.get_default_branch(remote_name)?;
        (default_branch, import_stats)
    };

    // Warn unmatched exact patterns, and record the first matching branch as
    // the working branch. If there are no matching exact patterns, use the
    // default branch of the remote.
    let mut missing_branches = vec![];
    let mut working_branch = None;
    let bookmark_matcher = bookmark_expr.to_matcher();
    let exact_bookmarks = bookmark_expr
        .exact_strings()
        .filter(|name| bookmark_matcher.is_match(name)) // exclude negative patterns
        .map(RefName::new);
    for name in exact_bookmarks {
        let symbol = name.to_remote_symbol(remote_name);
        if tx.repo().view().get_remote_bookmark(symbol).is_absent() {
            missing_branches.push(name);
        } else if working_branch.is_none() {
            working_branch = Some(name);
        }
    }
    if working_branch.is_none() {
        working_branch = default_branch.as_deref().filter(|name| {
            let symbol = name.to_remote_symbol(remote_name);
            tx.repo().view().get_remote_bookmark(symbol).is_present()
        });
    }
    if !missing_branches.is_empty() {
        writeln!(
            ui.warning_default(),
            "No matching branches found on remote: {}",
            missing_branches
                .iter()
                .map(|name| name.as_symbol())
                .join(", ")
        )?;
    }

    let working_is_default = working_branch == default_branch.as_deref();
    if let Some(name) = working_branch
        && working_is_default
        && should_track_default
    {
        // For convenience, create local bookmark as Git would do.
        let remote_symbol = name.to_remote_symbol(remote_name);
        tx.repo_mut().track_remote_bookmark(remote_symbol)?;
    }
    print_git_import_stats(ui, tx.repo(), &import_stats, true)?;
    if git_settings.auto_local_bookmark && !should_track_default {
        writeln!(
            ui.hint_default(),
            "`git.track-default-bookmark-on-clone=false` has no effect if \
             `git.auto-local-bookmark` is enabled."
        )?;
    }
    tx.finish(ui, "fetch from git remote into empty repo")?;
    Ok((working_branch.map(ToOwned::to_owned), working_is_default))
}
