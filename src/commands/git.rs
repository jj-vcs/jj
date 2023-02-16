use std::fs;
use std::io::{Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use clap::{ArgGroup, Subcommand};
use itertools::Itertools;
use jujutsu_lib::backend::ObjectId;
use jujutsu_lib::git::{self, GitFetchError, GitRefUpdate};
use jujutsu_lib::op_store::{BranchTarget, RefTarget};
use jujutsu_lib::refs::{classify_branch_push_action, BranchPushAction, BranchPushUpdate};
use jujutsu_lib::repo::{Repo, RepoRef};
use jujutsu_lib::settings::UserSettings;
use jujutsu_lib::store::Store;
use jujutsu_lib::view::View;
use jujutsu_lib::workspace::Workspace;
use maplit::hashset;

use crate::cli_util::{
    print_failed_git_export, short_change_hash, short_commit_hash, user_error, CommandError,
    CommandHelper, RevisionArg, WorkspaceCommandHelper,
};
use crate::commands::make_branch_term;
use crate::progress::Progress;
use crate::ui::Ui;

/// Commands for working with the underlying Git repo
///
/// For a comparison with Git, including a table of commands, see
/// https://github.com/martinvonz/jj/blob/main/docs/git-comparison.md.
#[derive(Subcommand, Clone, Debug)]
pub enum GitCommands {
    #[command(subcommand)]
    Remote(GitRemoteCommands),
    Fetch(GitFetchArgs),
    Clone(GitCloneArgs),
    Push(GitPushArgs),
    Import(GitImportArgs),
    Export(GitExportArgs),
}

/// Manage Git remotes
///
/// The Git repo will be a bare git repo stored inside the `.jj/` directory.
#[derive(Subcommand, Clone, Debug)]
pub enum GitRemoteCommands {
    Add(GitRemoteAddArgs),
    Remove(GitRemoteRemoveArgs),
    Rename(GitRemoteRenameArgs),
    List(GitRemoteListArgs),
}

/// Add a Git remote
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteAddArgs {
    /// The remote's name
    remote: String,
    /// The remote's URL
    url: String,
}

/// Remove a Git remote and forget its branches
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteRemoveArgs {
    /// The remote's name
    remote: String,
}

/// Rename a Git remote
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteRenameArgs {
    /// The name of an existing remote
    old: String,
    /// The desired name for `old`
    new: String,
}

/// List Git remotes
#[derive(clap::Args, Clone, Debug)]
pub struct GitRemoteListArgs {}

/// Fetch from a Git remote
#[derive(clap::Args, Clone, Debug)]
pub struct GitFetchArgs {
    /// The remote to fetch from (only named remotes are supported, can be
    /// repeated)
    #[arg(long = "remote", value_name = "remote")]
    remotes: Vec<String>,
}

/// Create a new repo backed by a clone of a Git repo
///
/// The Git repo will be a bare git repo stored inside the `.jj/` directory.
#[derive(clap::Args, Clone, Debug)]
pub struct GitCloneArgs {
    /// URL or path of the Git repo to clone
    #[arg(value_hint = clap::ValueHint::DirPath)]
    source: String,
    /// The directory to write the Jujutsu repo to
    #[arg(value_hint = clap::ValueHint::DirPath)]
    destination: Option<String>,
}

/// Push to a Git remote
///
/// By default, pushes any branches pointing to `@`, or `@-` if no branches
/// point to `@`. Use `--branch` to push specific branches. Use `--all` to push
/// all branches. Use `--change` to generate branch names based on the change
/// IDs of specific commits.
#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("what").args(&["branch", "all", "change"])))]
pub struct GitPushArgs {
    /// The remote to push to (only named remotes are supported)
    #[arg(long)]
    remote: Option<String>,
    /// Push only this branch (can be repeated)
    #[arg(long, short)]
    branch: Vec<String>,
    /// Push all branches
    #[arg(long)]
    all: bool,
    /// Push this commit by creating a branch based on its change ID (can be
    /// repeated)
    #[arg(long)]
    change: Vec<RevisionArg>,
    /// Only display what will change on the remote
    #[arg(long)]
    dry_run: bool,
}

/// Update repo with changes made in the underlying Git repo
#[derive(clap::Args, Clone, Debug)]
pub struct GitImportArgs {}

/// Update the underlying Git repo with changes made in the repo
#[derive(clap::Args, Clone, Debug)]
pub struct GitExportArgs {}

fn get_git_repo(store: &Store) -> Result<git2::Repository, CommandError> {
    match store.git_repo() {
        None => Err(user_error("The repo is not backed by a git repo")),
        Some(git_repo) => Ok(git_repo),
    }
}

fn cmd_git_remote_add(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteAddArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    if git_repo.find_remote(&args.remote).is_ok() {
        return Err(user_error("Remote already exists"));
    }
    git_repo
        .remote(&args.remote, &args.url)
        .map_err(|err| user_error(err.to_string()))?;
    Ok(())
}

fn cmd_git_remote_remove(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteRemoveArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    if git_repo.find_remote(&args.remote).is_err() {
        return Err(user_error("Remote doesn't exist"));
    }
    git_repo
        .remote_delete(&args.remote)
        .map_err(|err| user_error(err.to_string()))?;
    let mut branches_to_delete = vec![];
    for (branch, target) in repo.view().branches() {
        if target.remote_targets.contains_key(&args.remote) {
            branches_to_delete.push(branch.clone());
        }
    }
    if !branches_to_delete.is_empty() {
        let mut tx =
            workspace_command.start_transaction(&format!("remove git remote {}", &args.remote));
        for branch in branches_to_delete {
            tx.mut_repo().remove_remote_branch(&branch, &args.remote);
        }
        tx.finish(ui)?;
    }
    Ok(())
}

fn cmd_git_remote_rename(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRemoteRenameArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    if git_repo.find_remote(&args.old).is_err() {
        return Err(user_error("Remote doesn't exist"));
    }
    git_repo
        .remote_rename(&args.old, &args.new)
        .map_err(|err| user_error(err.to_string()))?;
    let mut tx = workspace_command
        .start_transaction(&format!("rename git remote {} to {}", &args.old, &args.new));
    tx.mut_repo().rename_remote(&args.old, &args.new);
    if tx.mut_repo().has_changes() {
        tx.finish(ui)?;
    }
    Ok(())
}

fn cmd_git_remote_list(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitRemoteListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    for remote_name in git_repo.remotes()?.iter().flatten() {
        let remote = git_repo.find_remote(remote_name)?;
        writeln!(ui, "{} {}", remote_name, remote.url().unwrap_or("<no URL>"))?;
    }
    Ok(())
}

#[tracing::instrument(skip(ui, command))]
fn cmd_git_fetch(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitFetchArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let remotes = if args.remotes.is_empty() {
        const KEY: &str = "git.fetch";
        let config = command.settings().config();
        config
            .get(KEY)
            .or_else(|_| config.get_string(KEY).map(|r| vec![r]))?
    } else {
        args.remotes.clone()
    };
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    let mut tx = workspace_command.start_transaction(&format!(
        "fetch from git remote(s) {}",
        remotes.iter().join(",")
    ));
    for remote in remotes {
        with_remote_callbacks(ui, |cb| {
            git::fetch(
                tx.mut_repo(),
                &git_repo,
                &remote,
                cb,
                &command.settings().git_settings(),
            )
        })
        .map_err(|err| user_error(err.to_string()))?;
    }
    tx.finish(ui)?;
    Ok(())
}

fn absolute_git_source(cwd: &Path, source: &str) -> String {
    // Git appears to turn URL-like source to absolute path if local git directory
    // exits, and fails because '$PWD/https' is unsupported protocol. Since it would
    // be tedious to copy the exact git (or libgit2) behavior, we simply assume a
    // source containing ':' is a URL, SSH remote, or absolute path with Windows
    // drive letter.
    if !source.contains(':') && Path::new(source).exists() {
        // It's less likely that cwd isn't utf-8, so just fall back to original source.
        cwd.join(source)
            .into_os_string()
            .into_string()
            .unwrap_or_else(|_| source.to_owned())
    } else {
        source.to_owned()
    }
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

fn cmd_git_clone(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitCloneArgs,
) -> Result<(), CommandError> {
    if command.global_args().repository.is_some() {
        return Err(user_error("'--repository' cannot be used with 'git clone'"));
    }
    let source = absolute_git_source(command.cwd(), &args.source);
    let wc_path_str = args
        .destination
        .as_deref()
        .or_else(|| clone_destination_for_source(&source))
        .ok_or_else(|| user_error("No destination specified and wasn't able to guess it"))?;
    let wc_path = command.cwd().join(wc_path_str);
    let wc_path_existed = wc_path.exists();
    if wc_path_existed {
        if !is_empty_dir(&wc_path) {
            return Err(user_error(
                "Destination path exists and is not an empty directory",
            ));
        }
    } else {
        fs::create_dir(&wc_path).unwrap();
    }

    let clone_result = do_git_clone(ui, command, &source, &wc_path);
    if clone_result.is_err() {
        // Canonicalize because fs::remove_dir_all() doesn't seem to like e.g.
        // `/some/path/.`
        let canonical_wc_path = wc_path.canonicalize().unwrap();
        if let Err(err) = fs::remove_dir_all(canonical_wc_path.join(".jj")).and_then(|_| {
            if !wc_path_existed {
                fs::remove_dir(&canonical_wc_path)
            } else {
                Ok(())
            }
        }) {
            writeln!(
                ui,
                "Failed to clean up {}: {}",
                canonical_wc_path.display(),
                err
            )
            .ok();
        }
    }

    if let (mut workspace_command, Some(default_branch)) = clone_result? {
        let default_branch_target = workspace_command
            .repo()
            .view()
            .get_remote_branch(&default_branch, "origin");
        if let Some(RefTarget::Normal(commit_id)) = default_branch_target {
            let mut checkout_tx =
                workspace_command.start_transaction("check out git remote's default branch");
            if let Ok(commit) = checkout_tx.base_repo().store().get_commit(&commit_id) {
                checkout_tx.check_out(&commit)?;
            }
            checkout_tx.finish(ui)?;
        }
    }
    Ok(())
}

fn do_git_clone(
    ui: &mut Ui,
    command: &CommandHelper,
    source: &str,
    wc_path: &Path,
) -> Result<(WorkspaceCommandHelper, Option<String>), CommandError> {
    let (workspace, repo) = Workspace::init_internal_git(command.settings(), wc_path)?;
    let git_repo = get_git_repo(repo.store())?;
    writeln!(ui, r#"Fetching into new repo in "{}""#, wc_path.display())?;
    let mut workspace_command = command.for_loaded_repo(ui, workspace, repo)?;
    let remote_name = "origin";
    git_repo.remote(remote_name, source).unwrap();
    let mut fetch_tx = workspace_command.start_transaction("fetch from git remote into empty repo");

    let maybe_default_branch = with_remote_callbacks(ui, |cb| {
        git::fetch(
            fetch_tx.mut_repo(),
            &git_repo,
            remote_name,
            cb,
            &command.settings().git_settings(),
        )
    })
    .map_err(|err| match err {
        GitFetchError::NoSuchRemote(_) => {
            panic!("shouldn't happen as we just created the git remote")
        }
        GitFetchError::InternalGitError(err) => user_error(format!("Fetch failed: {err}")),
    })?;
    fetch_tx.finish(ui)?;
    Ok((workspace_command, maybe_default_branch))
}

#[allow(clippy::explicit_auto_deref)] // https://github.com/rust-lang/rust-clippy/issues/9763
fn with_remote_callbacks<T>(ui: &mut Ui, f: impl FnOnce(git::RemoteCallbacks<'_>) -> T) -> T {
    let mut ui = Mutex::new(ui);
    let mut callback = None;
    if ui.get_mut().unwrap().use_progress_indicator() {
        let mut progress = Progress::new(Instant::now());
        let ui = &ui;
        callback = Some(move |x: &git::Progress| {
            _ = progress.update(Instant::now(), x, *ui.lock().unwrap());
        });
    }
    let mut callbacks = git::RemoteCallbacks::default();
    callbacks.progress = callback
        .as_mut()
        .map(|x| x as &mut dyn FnMut(&git::Progress));
    let mut get_ssh_key = get_ssh_key; // Coerce to unit fn type
    callbacks.get_ssh_key = Some(&mut get_ssh_key);
    let mut get_pw = |url: &str, _username: &str| {
        pinentry_get_pw(url).or_else(|| terminal_get_pw(*ui.lock().unwrap(), url))
    };
    callbacks.get_password = Some(&mut get_pw);
    let mut get_user_pw = |url: &str| {
        let ui = &mut *ui.lock().unwrap();
        Some((terminal_get_username(ui, url)?, terminal_get_pw(ui, url)?))
    };
    callbacks.get_username_password = Some(&mut get_user_pw);
    f(callbacks)
}

fn terminal_get_username(ui: &mut Ui, url: &str) -> Option<String> {
    ui.prompt(&format!("Username for {url}")).ok()
}

fn terminal_get_pw(ui: &mut Ui, url: &str) -> Option<String> {
    ui.prompt_password(&format!("Passphrase for {url}: ")).ok()
}

fn pinentry_get_pw(url: &str) -> Option<String> {
    let mut pinentry = Command::new("pinentry")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    #[rustfmt::skip]
    pinentry
        .stdin
        .take()
        .unwrap()
        .write_all(
            format!(
                "SETTITLE jj passphrase\n\
                 SETDESC Enter passphrase for {url}\n\
                 SETPROMPT Passphrase:\n\
                 GETPIN\n"
            )
            .as_bytes(),
        )
        .ok()?;
    let mut out = String::new();
    pinentry
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .ok()?;
    _ = pinentry.wait();
    for line in out.split('\n') {
        if !line.starts_with("D ") {
            continue;
        }
        let (_, encoded) = line.split_at(2);
        return decode_assuan_data(encoded);
    }
    None
}

// https://www.gnupg.org/documentation/manuals/assuan/Server-responses.html#Server-responses
fn decode_assuan_data(encoded: &str) -> Option<String> {
    let encoded = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        if encoded[i] != b'%' {
            decoded.push(encoded[i]);
            i += 1;
            continue;
        }
        i += 1;
        let byte =
            u8::from_str_radix(std::str::from_utf8(encoded.get(i..i + 2)?).ok()?, 16).ok()?;
        decoded.push(byte);
        i += 2;
    }
    String::from_utf8(decoded).ok()
}

#[tracing::instrument]
fn get_ssh_key(_username: &str) -> Option<PathBuf> {
    let home_dir = std::env::var("HOME").ok()?;
    let key_path = std::path::Path::new(&home_dir).join(".ssh").join("id_rsa");
    if key_path.is_file() {
        tracing::debug!(path = ?key_path, "found ssh key");
        Some(key_path)
    } else {
        tracing::debug!(path = ?key_path, "no ssh key found");
        None
    }
}

fn cmd_git_push(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitPushArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let remote = if let Some(name) = &args.remote {
        name.clone()
    } else {
        command.settings().config().get("git.push")?
    };
    let mut tx;
    let mut branch_updates = vec![];
    let mut seen_branches = hashset! {};
    if args.all {
        // TODO: Is it useful to warn about conflicted branches?
        for (branch_name, branch_target) in workspace_command.repo().view().branches() {
            if !seen_branches.insert(branch_name.clone()) {
                continue;
            }
            let push_action = classify_branch_push_action(branch_target, &remote);
            match push_action {
                BranchPushAction::AlreadyMatches => {}
                BranchPushAction::LocalConflicted => {}
                BranchPushAction::RemoteConflicted => {}
                BranchPushAction::Update(update) => {
                    branch_updates.push((branch_name.clone(), update));
                }
            }
        }
        tx = workspace_command
            .start_transaction(&format!("push all branches to git remote {}", &remote));
    } else if !args.branch.is_empty() {
        for branch_name in &args.branch {
            if !seen_branches.insert(branch_name.clone()) {
                continue;
            }
            if let Some(update) = branch_updates_for_push(
                workspace_command.repo().as_repo_ref(),
                &remote,
                branch_name,
            )? {
                branch_updates.push((branch_name.clone(), update));
            } else {
                writeln!(
                    ui,
                    "Branch {}@{} already matches {}",
                    branch_name, &remote, branch_name
                )?;
            }
        }
        tx = workspace_command.start_transaction(&format!(
            "push {} to git remote {}",
            make_branch_term(&args.branch),
            &remote
        ));
    } else if !args.change.is_empty() {
        // TODO: Allow specifying --branch and --change at the same time
        let commits: Vec<_> = args
            .change
            .iter()
            .map(|change_str| workspace_command.resolve_single_rev(change_str))
            .try_collect()?;
        tx = workspace_command.start_transaction(&format!(
            "push {} {} to git remote {}",
            if commits.len() > 1 {
                "changes"
            } else {
                "change"
            },
            commits.iter().map(|c| c.change_id().hex()).join(", "),
            &remote
        ));
        for (change_str, commit) in std::iter::zip(args.change.iter(), commits) {
            let mut branch_name = format!(
                "{}{}",
                command.settings().push_branch_prefix(),
                commit.change_id().hex()
            );
            if !seen_branches.insert(branch_name.clone()) {
                continue;
            }
            let view = tx.base_repo().view();
            if view.get_local_branch(&branch_name).is_none() {
                // A local branch with the full change ID doesn't exist already, so use the
                // short ID if it's not ambiguous (which it shouldn't be most of the time).
                let short_change_id = short_change_hash(commit.change_id());
                if tx
                    .base_workspace_helper()
                    .resolve_single_rev(&short_change_id)
                    .is_ok()
                {
                    // Short change ID is not ambiguous, so update the branch name to use it.
                    branch_name = format!(
                        "{}{}",
                        command.settings().push_branch_prefix(),
                        short_change_id
                    );
                };
            }
            if view.get_local_branch(&branch_name).is_none() {
                writeln!(
                    ui,
                    "Creating branch {} for revision {}",
                    branch_name,
                    change_str.deref()
                )?;
            }
            tx.mut_repo()
                .set_local_branch(branch_name.clone(), RefTarget::Normal(commit.id().clone()));
            if let Some(update) =
                branch_updates_for_push(tx.mut_repo().as_repo_ref(), &remote, &branch_name)?
            {
                branch_updates.push((branch_name.clone(), update));
            } else {
                writeln!(
                    ui,
                    "Branch {}@{} already matches {}",
                    branch_name, &remote, branch_name
                )?;
            }
        }
    } else {
        match workspace_command
            .repo()
            .view()
            .get_wc_commit_id(workspace_command.workspace_id())
        {
            None => {
                return Err(user_error("Nothing checked out in this workspace"));
            }
            Some(wc_commit) => {
                fn find_branches_targeting<'a>(
                    view: &'a View,
                    target: &RefTarget,
                ) -> Vec<(&'a String, &'a BranchTarget)> {
                    view.branches()
                        .iter()
                        .filter(|(_, branch_target)| {
                            branch_target.local_target.as_ref() == Some(target)
                        })
                        .collect()
                }

                // Search for branches targeting @
                let mut branches = find_branches_targeting(
                    workspace_command.repo().view(),
                    &RefTarget::Normal(wc_commit.clone()),
                );
                if branches.is_empty() {
                    // Try @- instead if it has exactly one parent, such as after `jj squash`
                    let commit = workspace_command.repo().store().get_commit(wc_commit)?;
                    if let [parent] = commit.parent_ids() {
                        branches = find_branches_targeting(
                            workspace_command.repo().view(),
                            &RefTarget::Normal(parent.clone()),
                        );
                    }
                }
                if branches.is_empty() {
                    return Err(user_error("No current branch."));
                }
                for (branch_name, branch_target) in branches {
                    if !seen_branches.insert(branch_name.clone()) {
                        continue;
                    }
                    let push_action = classify_branch_push_action(branch_target, &remote);
                    match push_action {
                        BranchPushAction::AlreadyMatches => {}
                        BranchPushAction::LocalConflicted => {}
                        BranchPushAction::RemoteConflicted => {}
                        BranchPushAction::Update(update) => {
                            branch_updates.push((branch_name.clone(), update));
                        }
                    }
                }
            }
        }
        tx = workspace_command.start_transaction(&format!(
            "push current branch(es) to git remote {}",
            &remote
        ));
    }
    drop(seen_branches);

    if branch_updates.is_empty() {
        writeln!(ui, "Nothing changed.")?;
        return Ok(());
    }

    let repo = tx.base_repo();

    let mut ref_updates = vec![];
    let mut new_heads = vec![];
    let mut force_pushed_branches = hashset! {};
    for (branch_name, update) in &branch_updates {
        let qualified_name = format!("refs/heads/{branch_name}");
        if let Some(new_target) = &update.new_target {
            new_heads.push(new_target.clone());
            let force = match &update.old_target {
                None => false,
                Some(old_target) => !repo.index().is_ancestor(old_target, new_target),
            };
            if force {
                force_pushed_branches.insert(branch_name.to_string());
            }
            ref_updates.push(GitRefUpdate {
                qualified_name,
                force,
                new_target: Some(new_target.clone()),
            });
        } else {
            ref_updates.push(GitRefUpdate {
                qualified_name,
                force: false,
                new_target: None,
            });
        }
    }

    // Check if there are conflicts in any commits we're about to push that haven't
    // already been pushed.
    let mut old_heads = vec![];
    for branch_target in repo.view().branches().values() {
        if let Some(old_head) = branch_target.remote_targets.get(&remote) {
            old_heads.extend(old_head.adds());
        }
    }
    if old_heads.is_empty() {
        old_heads.push(repo.store().root_commit_id().clone());
    }
    for index_entry in repo.index().walk_revs(&new_heads, &old_heads) {
        let commit = repo.store().get_commit(&index_entry.commit_id())?;
        let mut reasons = vec![];
        if commit.description().is_empty() {
            reasons.push("it has no description");
        }
        if commit.author().name == UserSettings::user_name_placeholder()
            || commit.author().email == UserSettings::user_email_placeholder()
            || commit.committer().name == UserSettings::user_name_placeholder()
            || commit.committer().email == UserSettings::user_email_placeholder()
        {
            reasons.push("it has no author and/or committer set");
        }
        if commit.tree().has_conflict() {
            reasons.push("it has conflicts");
        }
        if !reasons.is_empty() {
            return Err(user_error(format!(
                "Won't push commit {} since {}",
                short_commit_hash(commit.id()),
                reasons.join(" and ")
            )));
        }
    }

    writeln!(ui, "Branch changes to push to {}:", &remote)?;
    for (branch_name, update) in &branch_updates {
        match (&update.old_target, &update.new_target) {
            (Some(old_target), Some(new_target)) => {
                if force_pushed_branches.contains(branch_name) {
                    writeln!(
                        ui,
                        "  Force branch {branch_name} from {} to {}",
                        short_commit_hash(old_target),
                        short_commit_hash(new_target)
                    )?;
                } else {
                    writeln!(
                        ui,
                        "  Move branch {branch_name} from {} to {}",
                        short_commit_hash(old_target),
                        short_commit_hash(new_target)
                    )?;
                }
            }
            (Some(old_target), None) => {
                writeln!(
                    ui,
                    "  Delete branch {branch_name} from {}",
                    short_commit_hash(old_target)
                )?;
            }
            (None, Some(new_target)) => {
                writeln!(
                    ui,
                    "  Add branch {branch_name} to {}",
                    short_commit_hash(new_target)
                )?;
            }
            (None, None) => {
                panic!("Not pushing any change to branch {branch_name}");
            }
        }
    }

    if args.dry_run {
        writeln!(ui, "Dry-run requested, not pushing.")?;
        return Ok(());
    }

    let git_repo = get_git_repo(repo.store())?;
    with_remote_callbacks(ui, |cb| {
        git::push_updates(&git_repo, &remote, &ref_updates, cb)
    })
    .map_err(|err| user_error(err.to_string()))?;
    git::import_refs(tx.mut_repo(), &git_repo, &command.settings().git_settings())?;
    tx.finish(ui)?;
    Ok(())
}

fn branch_updates_for_push(
    repo: RepoRef,
    remote_name: &str,
    branch_name: &str,
) -> Result<Option<BranchPushUpdate>, CommandError> {
    let maybe_branch_target = repo.view().get_branch(branch_name);
    let branch_target = maybe_branch_target
        .ok_or_else(|| user_error(format!("Branch {branch_name} doesn't exist")))?;
    let push_action = classify_branch_push_action(branch_target, remote_name);

    match push_action {
        BranchPushAction::AlreadyMatches => Ok(None),
        BranchPushAction::LocalConflicted => {
            Err(user_error(format!("Branch {branch_name} is conflicted")))
        }
        BranchPushAction::RemoteConflicted => Err(user_error(format!(
            "Branch {branch_name}@{remote_name} is conflicted"
        ))),
        BranchPushAction::Update(update) => Ok(Some(update)),
    }
}

fn cmd_git_import(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitImportArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    let mut tx = workspace_command.start_transaction("import git refs");
    git::import_refs(tx.mut_repo(), &git_repo, &command.settings().git_settings())?;
    tx.finish(ui)?;
    Ok(())
}

fn cmd_git_export(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GitExportArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let git_repo = get_git_repo(repo.store())?;
    let mut tx = workspace_command.start_transaction("export git refs");
    let failed_branches = git::export_refs(tx.mut_repo(), &git_repo)?;
    tx.finish(ui)?;
    print_failed_git_export(ui, &failed_branches)?;
    Ok(())
}

pub fn cmd_git(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitCommands,
) -> Result<(), CommandError> {
    match subcommand {
        GitCommands::Fetch(command_matches) => cmd_git_fetch(ui, command, command_matches),
        GitCommands::Clone(command_matches) => cmd_git_clone(ui, command, command_matches),
        GitCommands::Remote(GitRemoteCommands::Add(command_matches)) => {
            cmd_git_remote_add(ui, command, command_matches)
        }
        GitCommands::Remote(GitRemoteCommands::Remove(command_matches)) => {
            cmd_git_remote_remove(ui, command, command_matches)
        }
        GitCommands::Remote(GitRemoteCommands::Rename(command_matches)) => {
            cmd_git_remote_rename(ui, command, command_matches)
        }
        GitCommands::Remote(GitRemoteCommands::List(command_matches)) => {
            cmd_git_remote_list(ui, command, command_matches)
        }
        GitCommands::Push(command_matches) => cmd_git_push(ui, command, command_matches),
        GitCommands::Import(command_matches) => cmd_git_import(ui, command, command_matches),
        GitCommands::Export(command_matches) => cmd_git_export(ui, command, command_matches),
    }
}
