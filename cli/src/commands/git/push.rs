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

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::io::Write as _;

use clap::ArgGroup;
use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::git;
use jj_lib::git::GitBranchPushTargets;
use jj_lib::git::GitPushStats;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::refs::classify_bookmark_push_action;
use jj_lib::refs::BookmarkPushAction;
use jj_lib::refs::BookmarkPushUpdate;
use jj_lib::refs::LocalAndRemoteRef;
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::signing::SignBehavior;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use crate::cli_util::has_tracked_remote_bookmarks;
use crate::cli_util::short_change_hash;
use crate::cli_util::short_commit_hash;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::cli_error;
use crate::command_error::cli_error_with_message;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::CommandError;
use crate::commands::git::get_single_remote;
use crate::complete;
use crate::formatter::Formatter;
use crate::git_util::with_remote_git_callbacks;
use crate::revset_util::parse_bookmark_name;
use crate::ui::Ui;

/// Push to a Git remote
///
/// By default, pushes tracking bookmarks pointing to
/// `remote_bookmarks(remote=<remote>)..@`. Use `--bookmark` to push specific
/// bookmarks. Use `--all` to push all bookmarks. Use `--change` to generate
/// bookmark names based on the change IDs of specific commits.
///
/// Unlike in Git, the remote to push to is not derived from the tracked remote
/// bookmarks. Use `--remote` to select the remote Git repository by name. There
/// is no option to push to multiple remotes.
///
/// Before the command actually moves, creates, or deletes a remote bookmark, it
/// makes several [safety checks]. If there is a problem, you may need to run
/// `jj git fetch --remote <remote name>` and/or resolve some [bookmark
/// conflicts].
///
/// [safety checks]:
///     https://jj-vcs.github.io/jj/latest/bookmarks/#pushing-bookmarks-safety-checks
///
/// [bookmark conflicts]:
///     https://jj-vcs.github.io/jj/latest/bookmarks/#conflicts

#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("specific").args(&["bookmark", "change", "revisions", "named"]).multiple(true)))]
#[command(group(ArgGroup::new("what").args(&["all", "tracked"]).conflicts_with("specific")))]
pub struct GitPushArgs {
    /// The remote to push to (only named remotes are supported)
    ///
    /// This defaults to the `git.push` setting. If that is not configured, and
    /// if there are multiple remotes, the remote named "origin" will be used.
    #[arg(long, add = ArgValueCandidates::new(complete::git_remotes))]
    remote: Option<RemoteNameBuf>,
    /// Push only this bookmark, or bookmarks matching a pattern (can be
    /// repeated)
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select bookmarks by [wildcard pattern].
    ///
    /// [wildcard pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets#string-patterns
    #[arg(
        long, short,
        alias = "branch",
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::local_bookmarks),
    )]
    bookmark: Vec<StringPattern>,
    /// Push all bookmarks (including new bookmarks)
    #[arg(long)]
    all: bool,
    /// Push all tracked bookmarks
    ///
    /// This usually means that the bookmark was already pushed to or fetched
    /// from the [relevant remote].
    ///
    /// [relevant remote]:
    ///     https://jj-vcs.github.io/jj/latest/bookmarks#remotes-and-tracked-bookmarks
    #[arg(long)]
    tracked: bool,
    /// Push all deleted bookmarks
    ///
    /// Only tracked bookmarks can be successfully deleted on the remote. A
    /// warning will be printed if any untracked bookmarks on the remote
    /// correspond to missing local bookmarks.
    #[arg(long, conflicts_with = "specific")]
    deleted: bool,
    /// Allow pushing new bookmarks
    ///
    /// Newly-created remote bookmarks will be tracked automatically.
    ///
    /// This can also be turned on by the `git.push-new-bookmarks` setting. If
    /// it's set to `true`, `--allow-new` is no-op.
    #[arg(long, short = 'N', conflicts_with = "what")]
    allow_new: bool,
    /// Allow pushing commits with empty descriptions
    #[arg(long)]
    allow_empty_description: bool,
    /// Allow pushing commits that are private
    ///
    /// The set of private commits can be configured by the
    /// `git.private-commits` setting. The default is `none()`, meaning all
    /// commits are eligible to be pushed.
    #[arg(long)]
    allow_private: bool,
    /// Push bookmarks pointing to these commits (can be repeated)
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        // While `-r` will often be used with mutable revisions, immutable
        // revisions can be useful as parts of revsets or to push
        // special-purpose branches.
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revisions: Vec<RevisionArg>,
    /// Push this commit by creating a bookmark based on its change ID (can be
    /// repeated)
    ///
    /// The created bookmark will be tracked automatically. Use the
    /// `git.push-bookmark-prefix` setting to change the prefix for generated
    /// names.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        // I'm guessing that `git push -c` is almost exclusively used with
        // recently created mutable revisions, even though it can in theory
        // be used with immutable ones as well. We can change it if the guess
        // turns out to be wrong.
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    change: Vec<RevisionArg>,
    /// Specify a new bookmark name and a revision to push under that name, e.g.
    /// '--named myfeature=@'
    ///
    /// Does not require --allow-new.
    #[arg(
        long,
        value_name = "NAME=REVISION",
        add = ArgValueCompleter::new(complete::branch_name_equals_any_revision)
    )]
    named: Vec<String>,
    /// Only display what will change on the remote
    #[arg(long)]
    dry_run: bool,
}

fn make_bookmark_term(bookmark_names: &[impl fmt::Display]) -> String {
    match bookmark_names {
        [bookmark_name] => format!("bookmark {bookmark_name}"),
        bookmark_names => format!("bookmarks {}", bookmark_names.iter().join(", ")),
    }
}

const DEFAULT_REMOTE: &RemoteName = RemoteName::new("origin");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BookmarkMoveDirection {
    Forward,
    Backward,
    Sideways,
}

pub fn cmd_git_push(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitPushArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let default_remote;
    let remote = if let Some(name) = &args.remote {
        name
    } else {
        default_remote = get_default_push_remote(ui, &workspace_command)?;
        &default_remote
    };

    let mut tx = workspace_command.start_transaction();
    let view = tx.repo().view();
    let tx_description;
    let mut bookmark_updates = vec![];
    if args.all {
        for (name, targets) in view.local_remote_bookmarks(remote) {
            let allow_new = true; // implied by --all
            match classify_bookmark_update(
                name.to_remote_symbol(remote),
                targets,
                allow_new,
                args.deleted,
            ) {
                Ok(Some(update)) => bookmark_updates.push((name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!(
            "push all bookmarks to git remote {remote}",
            remote = remote.as_symbol()
        );
    } else if args.tracked {
        for (name, targets) in view.local_remote_bookmarks(remote) {
            if !targets.remote_ref.is_tracked() {
                continue;
            }
            let allow_new = false; // doesn't matter
            match classify_bookmark_update(
                name.to_remote_symbol(remote),
                targets,
                allow_new,
                args.deleted,
            ) {
                Ok(Some(update)) => bookmark_updates.push((name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!(
            "push all tracked bookmarks to git remote {remote}",
            remote = remote.as_symbol()
        );
    } else if args.deleted {
        for (name, targets) in view.local_remote_bookmarks(remote) {
            if targets.local_target.is_present() {
                continue;
            }
            let allow_new = false; // doesn't matter
            let allow_delete = true;
            match classify_bookmark_update(
                name.to_remote_symbol(remote),
                targets,
                allow_new,
                allow_delete,
            ) {
                Ok(Some(update)) => bookmark_updates.push((name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!(
            "push all deleted bookmarks to git remote {remote}",
            remote = remote.as_symbol()
        );
    } else {
        let mut seen_bookmarks: HashSet<&RefName> = HashSet::new();

        // --change and --named don't move existing bookmarks. If they did, be
        // careful to not select old state by -r/--revisions and bookmark names.
        let bookmark_prefix = tx.settings().get_string("git.push-bookmark-prefix")?;
        let change_bookmark_names =
            create_change_bookmarks(ui, &mut tx, &args.change, &bookmark_prefix)?;
        let created_bookmark_names: Vec<RefNameBuf> = args
            .named
            .iter()
            .map(|name_revision| create_explicitly_named_bookmarks(ui, &mut tx, name_revision))
            .try_collect()?;
        let created_bookmarks = change_bookmark_names
            .iter()
            .chain(created_bookmark_names.iter())
            .map(|name| {
                let remote_symbol = name.to_remote_symbol(remote);
                let targets = LocalAndRemoteRef {
                    local_target: tx.repo().view().get_local_bookmark(name),
                    remote_ref: tx.repo().view().get_remote_bookmark(remote_symbol),
                };
                (remote_symbol, targets)
            });
        for (remote_symbol, targets) in created_bookmarks {
            let name = remote_symbol.name;
            if !seen_bookmarks.insert(name) {
                continue;
            }
            let allow_new = true; // --change implies creation of remote bookmark
            let allow_delete = false; // doesn't matter
            match classify_bookmark_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => bookmark_updates.push((name.to_owned(), update)),
                Ok(None) => writeln!(
                    ui.status(),
                    "Bookmark {remote_symbol} already matches {name}",
                    name = name.as_symbol()
                )?,
                Err(reason) => return Err(reason.into()),
            }
        }

        let view = tx.repo().view();
        let allow_new = args.allow_new || tx.settings().get("git.push-new-bookmarks")?;
        let bookmarks_by_name = find_bookmarks_to_push(view, &args.bookmark, remote)?;
        for &(name, targets) in &bookmarks_by_name {
            if !seen_bookmarks.insert(name) {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_delete = true; // named explicitly, allow delete without --delete
            match classify_bookmark_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => bookmark_updates.push((name.to_owned(), update)),
                Ok(None) => writeln!(
                    ui.status(),
                    "Bookmark {remote_symbol} already matches {name}",
                    name = name.as_symbol()
                )?,
                Err(reason) => return Err(reason.into()),
            }
        }

        let use_default_revset = args.bookmark.is_empty()
            && args.change.is_empty()
            && args.revisions.is_empty()
            && args.named.is_empty();
        let bookmarks_targeted = find_bookmarks_targeted_by_revisions(
            ui,
            tx.base_workspace_helper(),
            remote,
            &args.revisions,
            use_default_revset,
        )?;
        for &(name, targets) in &bookmarks_targeted {
            if !seen_bookmarks.insert(name) {
                continue;
            }
            let allow_delete = false;
            match classify_bookmark_update(
                name.to_remote_symbol(remote),
                targets,
                allow_new,
                allow_delete,
            ) {
                Ok(Some(update)) => bookmark_updates.push((name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }

        tx_description = format!(
            "push {names} to git remote {remote}",
            names = make_bookmark_term(
                &bookmark_updates
                    .iter()
                    .map(|(name, _)| name.as_symbol())
                    .collect_vec()
            ),
            remote = remote.as_symbol()
        );
    }
    if bookmark_updates.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let sign_behavior = if tx.settings().get_bool("git.sign-on-push")? {
        Some(SignBehavior::Own)
    } else {
        None
    };
    let commits_to_sign =
        validate_commits_ready_to_push(ui, &bookmark_updates, remote, &tx, args, sign_behavior)?;
    if !args.dry_run && !commits_to_sign.is_empty() {
        if let Some(sign_behavior) = sign_behavior {
            let num_updated_signatures = commits_to_sign.len();
            let num_rebased_descendants;
            (num_rebased_descendants, bookmark_updates) = sign_commits_before_push(
                &mut tx,
                commits_to_sign,
                sign_behavior,
                bookmark_updates,
            )?;
            if let Some(mut formatter) = ui.status_formatter() {
                writeln!(
                    formatter,
                    "Updated signatures of {num_updated_signatures} commits"
                )?;
                if num_rebased_descendants > 0 {
                    writeln!(
                        formatter,
                        "Rebased {num_rebased_descendants} descendant commits"
                    )?;
                }
            }
        }
    }

    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "Changes to push to {remote}:",
            remote = remote.as_symbol()
        )?;
        print_commits_ready_to_push(formatter.as_mut(), tx.repo(), &bookmark_updates)?;
    }

    if args.dry_run {
        writeln!(ui.status(), "Dry-run requested, not pushing.")?;
        return Ok(());
    }

    let targets = GitBranchPushTargets {
        branch_updates: bookmark_updates,
    };
    let git_settings = tx.settings().git_settings()?;
    let push_stats = with_remote_git_callbacks(ui, |cb| {
        git::push_branches(tx.repo_mut(), &git_settings, remote, &targets, cb)
    })?;
    process_push_stats(&push_stats)?;
    tx.finish(ui, tx_description)?;
    Ok(())
}

fn process_push_stats(push_stats: &GitPushStats) -> Result<(), CommandError> {
    if !push_stats.all_ok() {
        let mut error = user_error("Failed to push some bookmarks");
        if !push_stats.rejected.is_empty() {
            error.add_formatted_hint_with(|formatter| {
                writeln!(
                    formatter,
                    "The following references unexpectedly moved on the remote:"
                )?;
                for (reference, reason) in &push_stats.rejected {
                    write!(formatter, "  ")?;
                    write!(formatter.labeled("git_ref"), "{}", reference.as_symbol())?;
                    if let Some(r) = reason {
                        write!(formatter, " (reason: {r})")?;
                    }
                    writeln!(formatter)?;
                }
                Ok(())
            });
            error.add_hint(
                "Try fetching from the remote, then make the bookmark point to where you want it \
                 to be, and push again.",
            );
        }
        if !push_stats.remote_rejected.is_empty() {
            error.add_formatted_hint_with(|formatter| {
                writeln!(formatter, "The remote rejected the following updates:")?;
                for (reference, reason) in &push_stats.remote_rejected {
                    write!(formatter, "  ")?;
                    write!(formatter.labeled("git_ref"), "{}", reference.as_symbol())?;
                    if let Some(r) = reason {
                        write!(formatter, " (reason: {r})")?;
                    }
                    writeln!(formatter)?;
                }
                Ok(())
            });
            error.add_hint("Try checking if you have permission to push to all the bookmarks.");
        }
        Err(error)
    } else {
        Ok(())
    }
}

/// Validates that the commits that will be pushed are ready (have authorship
/// information, are not conflicted, etc.).
///
/// Returns the list of commits which need to be signed.
fn validate_commits_ready_to_push(
    ui: &Ui,
    bookmark_updates: &[(RefNameBuf, BookmarkPushUpdate)],
    remote: &RemoteName,
    tx: &WorkspaceCommandTransaction,
    args: &GitPushArgs,
    sign_behavior: Option<SignBehavior>,
) -> Result<Vec<Commit>, CommandError> {
    let workspace_helper = tx.base_workspace_helper();
    let repo = workspace_helper.repo();

    let new_heads = bookmark_updates
        .iter()
        .filter_map(|(_, update)| update.new_target.clone())
        .collect_vec();
    let old_heads = repo
        .view()
        .remote_bookmarks(remote)
        .flat_map(|(_, old_head)| old_head.target.added_ids())
        .cloned()
        .collect_vec();
    let commits_to_push = RevsetExpression::commits(old_heads)
        .union(workspace_helper.env().immutable_heads_expression())
        .range(&RevsetExpression::commits(new_heads));

    let settings = workspace_helper.settings();
    let private_revset_str = RevisionArg::from(settings.get_string("git.private-commits")?);
    let is_private = workspace_helper
        .parse_revset(ui, &private_revset_str)?
        .evaluate()?
        .containing_fn();
    let sign_settings = sign_behavior.map(|sign_behavior| {
        let mut sign_settings = settings.sign_settings();
        sign_settings.behavior = sign_behavior;
        sign_settings
    });

    let mut commits_to_sign = vec![];

    for commit in workspace_helper
        .attach_revset_evaluator(commits_to_push)
        .evaluate_to_commits()?
    {
        let commit = commit?;
        let mut reasons = vec![];
        if commit.description().is_empty() && !args.allow_empty_description {
            reasons.push("it has no description");
        }
        if commit.author().name.is_empty()
            || commit.author().name == UserSettings::USER_NAME_PLACEHOLDER
            || commit.author().email.is_empty()
            || commit.author().email == UserSettings::USER_EMAIL_PLACEHOLDER
            || commit.committer().name.is_empty()
            || commit.committer().name == UserSettings::USER_NAME_PLACEHOLDER
            || commit.committer().email.is_empty()
            || commit.committer().email == UserSettings::USER_EMAIL_PLACEHOLDER
        {
            reasons.push("it has no author and/or committer set");
        }
        if commit.has_conflict()? {
            reasons.push("it has conflicts");
        }
        let is_private = is_private(commit.id())?;
        if !args.allow_private && is_private {
            reasons.push("it is private");
        }
        if !reasons.is_empty() {
            let mut error = user_error(format!(
                "Won't push commit {} since {}",
                short_commit_hash(commit.id()),
                reasons.join(" and ")
            ));
            error.add_formatted_hint_with(|formatter| {
                write!(formatter, "Rejected commit: ")?;
                workspace_helper.write_commit_summary(formatter, &commit)?;
                Ok(())
            });
            if !args.allow_private && is_private {
                error.add_hint(format!(
                    "Configured git.private-commits: '{private_revset_str}'",
                ));
            }
            return Err(error);
        }
        if let Some(sign_settings) = &sign_settings {
            if !commit.is_signed() && sign_settings.should_sign(commit.store_commit()) {
                commits_to_sign.push(commit);
            }
        }
    }
    Ok(commits_to_sign)
}

/// Signs commits before pushing.
///
/// Returns the number of commits with rebased descendants and the updated list
/// of bookmark names and corresponding [`BookmarkPushUpdate`]s.
fn sign_commits_before_push(
    tx: &mut WorkspaceCommandTransaction,
    commits_to_sign: Vec<Commit>,
    sign_behavior: SignBehavior,
    bookmark_updates: Vec<(RefNameBuf, BookmarkPushUpdate)>,
) -> Result<(usize, Vec<(RefNameBuf, BookmarkPushUpdate)>), CommandError> {
    let commit_ids: IndexSet<CommitId> = commits_to_sign.iter().ids().cloned().collect();
    let mut old_to_new_commits_map: HashMap<CommitId, CommitId> = HashMap::new();
    let mut num_rebased_descendants = 0;
    tx.repo_mut()
        .transform_descendants(commit_ids.iter().cloned().collect_vec(), |rewriter| {
            let old_commit_id = rewriter.old_commit().id().clone();
            if commit_ids.contains(&old_commit_id) {
                let commit = rewriter
                    .reparent()
                    .set_sign_behavior(sign_behavior)
                    .write()?;
                old_to_new_commits_map.insert(old_commit_id, commit.id().clone());
            } else {
                num_rebased_descendants += 1;
                let commit = rewriter.reparent().write()?;
                old_to_new_commits_map.insert(old_commit_id, commit.id().clone());
            }
            Ok(())
        })?;

    let bookmark_updates = bookmark_updates
        .into_iter()
        .map(|(bookmark_name, update)| {
            (
                bookmark_name,
                BookmarkPushUpdate {
                    old_target: update.old_target,
                    new_target: update
                        .new_target
                        .map(|id| old_to_new_commits_map.get(&id).cloned().unwrap_or(id)),
                },
            )
        })
        .collect_vec();

    Ok((num_rebased_descendants, bookmark_updates))
}

fn print_commits_ready_to_push(
    formatter: &mut dyn Formatter,
    repo: &dyn Repo,
    bookmark_updates: &[(RefNameBuf, BookmarkPushUpdate)],
) -> io::Result<()> {
    let to_direction = |old_target: &CommitId, new_target: &CommitId| {
        assert_ne!(old_target, new_target);
        if repo.index().is_ancestor(old_target, new_target) {
            BookmarkMoveDirection::Forward
        } else if repo.index().is_ancestor(new_target, old_target) {
            BookmarkMoveDirection::Backward
        } else {
            BookmarkMoveDirection::Sideways
        }
    };

    for (bookmark_name, update) in bookmark_updates {
        match (&update.old_target, &update.new_target) {
            (Some(old_target), Some(new_target)) => {
                let bookmark_name = bookmark_name.as_symbol();
                let old = short_commit_hash(old_target);
                let new = short_commit_hash(new_target);
                // TODO(ilyagr): Add color. Once there is color, "Move bookmark ... sideways"
                // may read more naturally than "Move sideways bookmark ...".
                // Without color, it's hard to see at a glance if one bookmark
                // among many was moved sideways (say). TODO: People on Discord
                // suggest "Move bookmark ... forward by n commits",
                // possibly "Move bookmark ... sideways (X forward, Y back)".
                let msg = match to_direction(old_target, new_target) {
                    BookmarkMoveDirection::Forward => {
                        format!("Move forward bookmark {bookmark_name} from {old} to {new}")
                    }
                    BookmarkMoveDirection::Backward => {
                        format!("Move backward bookmark {bookmark_name} from {old} to {new}")
                    }
                    BookmarkMoveDirection::Sideways => {
                        format!("Move sideways bookmark {bookmark_name} from {old} to {new}")
                    }
                };
                writeln!(formatter, "  {msg}")?;
            }
            (Some(old_target), None) => {
                writeln!(
                    formatter,
                    "  Delete bookmark {bookmark_name} from {old}",
                    bookmark_name = bookmark_name.as_symbol(),
                    old = short_commit_hash(old_target)
                )?;
            }
            (None, Some(new_target)) => {
                writeln!(
                    formatter,
                    "  Add bookmark {bookmark_name} to {new}",
                    bookmark_name = bookmark_name.as_symbol(),
                    new = short_commit_hash(new_target)
                )?;
            }
            (None, None) => {
                panic!("Not pushing any change to bookmark {bookmark_name:?}");
            }
        }
    }
    Ok(())
}

fn get_default_push_remote(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
) -> Result<RemoteNameBuf, CommandError> {
    let settings = workspace_command.settings();
    if let Some(remote) = settings.get_string("git.push").optional()? {
        Ok(remote.into())
    } else if let Some(remote) = get_single_remote(workspace_command.repo().store())? {
        // similar to get_default_fetch_remotes
        if remote != DEFAULT_REMOTE {
            writeln!(
                ui.hint_default(),
                "Pushing to the only existing remote: {remote}",
                remote = remote.as_symbol()
            )?;
        }
        Ok(remote)
    } else {
        Ok(DEFAULT_REMOTE.to_owned())
    }
}

#[derive(Clone, Debug)]
struct RejectedBookmarkUpdateReason {
    message: String,
    hint: Option<String>,
}

impl RejectedBookmarkUpdateReason {
    fn print(&self, ui: &Ui) -> io::Result<()> {
        writeln!(ui.warning_default(), "{}", self.message)?;
        if let Some(hint) = &self.hint {
            writeln!(ui.hint_default(), "{hint}")?;
        }
        Ok(())
    }
}

impl From<RejectedBookmarkUpdateReason> for CommandError {
    fn from(reason: RejectedBookmarkUpdateReason) -> Self {
        let RejectedBookmarkUpdateReason { message, hint } = reason;
        let mut cmd_err = user_error(message);
        cmd_err.extend_hints(hint);
        cmd_err
    }
}

fn classify_bookmark_update(
    remote_symbol: RemoteRefSymbol<'_>,
    targets: LocalAndRemoteRef,
    allow_new: bool,
    allow_delete: bool,
) -> Result<Option<BookmarkPushUpdate>, RejectedBookmarkUpdateReason> {
    let push_action = classify_bookmark_push_action(targets);
    match push_action {
        BookmarkPushAction::AlreadyMatches => Ok(None),
        BookmarkPushAction::LocalConflicted => Err(RejectedBookmarkUpdateReason {
            message: format!(
                "Bookmark {name} is conflicted",
                name = remote_symbol.name.as_symbol()
            ),
            hint: Some(
                "Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up."
                    .to_owned(),
            ),
        }),
        BookmarkPushAction::RemoteConflicted => Err(RejectedBookmarkUpdateReason {
            message: format!("Bookmark {remote_symbol} is conflicted"),
            hint: Some("Run `jj git fetch` to update the conflicted remote bookmark.".to_owned()),
        }),
        BookmarkPushAction::RemoteUntracked => Err(RejectedBookmarkUpdateReason {
            message: format!("Non-tracking remote bookmark {remote_symbol} exists"),
            hint: Some(format!(
                "Run `jj bookmark track {remote_symbol}` to import the remote bookmark."
            )),
        }),
        BookmarkPushAction::Update(update) if update.old_target.is_none() && !allow_new => {
            Err(RejectedBookmarkUpdateReason {
                message: format!("Refusing to create new remote bookmark {remote_symbol}"),
                hint: Some(
                    "Use --allow-new to push new bookmark. Use --remote to specify the remote to \
                     push to."
                        .to_owned(),
                ),
            })
        }
        BookmarkPushAction::Update(update) if update.new_target.is_none() && !allow_delete => {
            Err(RejectedBookmarkUpdateReason {
                message: format!(
                    "Refusing to push deleted bookmark {name}",
                    name = remote_symbol.name.as_symbol(),
                ),
                hint: Some(
                    "Push deleted bookmarks with --deleted or forget the bookmark to suppress \
                     this warning."
                        .to_owned(),
                ),
            })
        }
        BookmarkPushAction::Update(update) => Ok(Some(update)),
    }
}

fn ensure_new_bookmark_name(view: &View, name: &RefName) -> Result<(), CommandError> {
    let symbol = name.as_symbol();
    if view.get_local_bookmark(name).is_present() {
        return Err(user_error_with_hint(
            format!("Bookmark already exists: {symbol}"),
            format!(
                "Use 'jj bookmark move' to move it, and 'jj git push -b {symbol} [--allow-new]' \
                 to push it"
            ),
        ));
    }
    if has_tracked_remote_bookmarks(view, name) {
        return Err(user_error_with_hint(
            format!("Tracked remote bookmarks exist for deleted bookmark: {symbol}"),
            format!(
                "Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark untrack \
                 'glob:{symbol}@*'` to disassociate them."
            ),
        ));
    }
    Ok(())
}

/// Creates a bookmark for a single `--named` argument and returns its name
///
/// The logic is not identical to that of `jj bookmark create` since we need to
/// make sure the new bookmark is safe to push.
fn create_explicitly_named_bookmarks(
    ui: &Ui,
    tx: &mut WorkspaceCommandTransaction<'_>,
    name_revision: &String,
) -> Result<RefNameBuf, CommandError> {
    let hint = "For example, `--named myfeature=@` is valid syntax";
    let Some((name_str, revision_str)) = name_revision.split_once('=') else {
        return Err(cli_error(format!(
            "Argument '{name_revision}' must include '=' and have the form NAME=REVISION"
        ))
        .hinted(hint));
    };
    if name_str.is_empty() || revision_str.is_empty() {
        return Err(cli_error(format!(
            "Argument '{name_revision}' must have the form NAME=REVISION, with both NAME and \
             REVISION non-empty"
        ))
        .hinted(hint));
    }
    let name = parse_bookmark_name(name_str).map_err(|err| {
        cli_error_with_message(
            format!("Could not parse '{name_str}' as a bookmark name"),
            err,
        )
        .hinted(hint)
    })?;
    ensure_new_bookmark_name(tx.repo().view(), &name)?;
    let revision = tx
        .base_workspace_helper()
        .resolve_single_rev(ui, &revision_str.to_string().into())?;
    tx.repo_mut()
        .set_local_bookmark_target(&name, RefTarget::normal(revision.id().clone()));
    Ok(name)
}

/// Creates bookmarks based on the change IDs.
fn create_change_bookmarks(
    ui: &Ui,
    tx: &mut WorkspaceCommandTransaction,
    changes: &[RevisionArg],
    bookmark_prefix: &str,
) -> Result<Vec<RefNameBuf>, CommandError> {
    if changes.is_empty() {
        // NOTE: we don't want resolve_some_revsets_default_single to fail if the
        // changes argument wasn't provided, so handle that
        return Ok(vec![]);
    }

    let mut bookmark_names = Vec::new();
    let all_commits: Vec<_> = tx
        .base_workspace_helper()
        .resolve_some_revsets_default_single(ui, changes)?
        .iter()
        .map(|id| tx.repo().store().get_commit(id))
        .try_collect()?;

    for commit in all_commits {
        let short_change_id = short_change_hash(commit.change_id());
        let name: RefNameBuf = format!("{bookmark_prefix}{short_change_id}").into();
        let target = RefTarget::normal(commit.id().clone());
        let view = tx.base_repo().view();
        if view.get_local_bookmark(&name) == &target {
            // Existing bookmark pointing to the commit, which is allowed
        } else {
            ensure_new_bookmark_name(view, &name)?;
            writeln!(
                ui.status(),
                "Creating bookmark {name} for revision {short_change_id}",
                name = name.as_symbol()
            )?;
            tx.repo_mut().set_local_bookmark_target(&name, target);
        }
        bookmark_names.push(name);
    }
    Ok(bookmark_names)
}

fn find_bookmarks_to_push<'a>(
    view: &'a View,
    bookmark_patterns: &[StringPattern],
    remote: &RemoteName,
) -> Result<Vec<(&'a RefName, LocalAndRemoteRef<'a>)>, CommandError> {
    let mut matching_bookmarks = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in bookmark_patterns {
        let mut matches = view
            .local_remote_bookmarks_matching(pattern, remote)
            .filter(|(_, targets)| {
                // If the remote exists but is not tracked, the absent local shouldn't
                // be considered a deleted bookmark.
                targets.local_target.is_present() || targets.remote_ref.is_tracked()
            })
            .peekable();
        if matches.peek().is_none() {
            unmatched_patterns.push(pattern);
        }
        matching_bookmarks.extend(matches);
    }
    match &unmatched_patterns[..] {
        [] => Ok(matching_bookmarks),
        [pattern] if pattern.is_exact() => Err(user_error(format!("No such bookmark: {pattern}"))),
        patterns => Err(user_error(format!(
            "No matching bookmarks for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}

fn find_bookmarks_targeted_by_revisions<'a>(
    ui: &Ui,
    workspace_command: &'a WorkspaceCommandHelper,
    remote: &RemoteName,
    revisions: &[RevisionArg],
    use_default_revset: bool,
) -> Result<Vec<(&'a RefName, LocalAndRemoteRef<'a>)>, CommandError> {
    let mut revision_commit_ids = HashSet::new();
    if use_default_revset {
        // remote_bookmarks(remote=<remote>)..@
        let workspace_name = workspace_command.workspace_name();
        let expression = RevsetExpression::remote_bookmarks(
            StringPattern::everything(),
            StringPattern::exact(remote),
            None,
        )
        .range(&RevsetExpression::working_copy(workspace_name.to_owned()))
        .intersection(&RevsetExpression::bookmarks(StringPattern::everything()));
        let mut commit_ids = workspace_command
            .attach_revset_evaluator(expression)
            .evaluate_to_commit_ids()?
            .peekable();
        if commit_ids.peek().is_none() {
            writeln!(
                ui.warning_default(),
                "No bookmarks found in the default push revset: \
                 remote_bookmarks(remote={remote})..@",
                remote = remote.as_symbol()
            )?;
        }
        for commit_id in commit_ids {
            revision_commit_ids.insert(commit_id?);
        }
    }
    for rev_arg in revisions {
        let mut expression = workspace_command.parse_revset(ui, rev_arg)?;
        expression.intersect_with(&RevsetExpression::bookmarks(StringPattern::everything()));
        let mut commit_ids = expression.evaluate_to_commit_ids()?.peekable();
        if commit_ids.peek().is_none() {
            writeln!(
                ui.warning_default(),
                "No bookmarks point to the specified revisions: {rev_arg}"
            )?;
        }
        for commit_id in commit_ids {
            revision_commit_ids.insert(commit_id?);
        }
    }
    let bookmarks_targeted = workspace_command
        .repo()
        .view()
        .local_remote_bookmarks(remote)
        .filter(|(_, targets)| {
            let mut local_ids = targets.local_target.added_ids();
            local_ids.any(|id| revision_commit_ids.contains(id))
        })
        .collect_vec();
    Ok(bookmarks_targeted)
}
