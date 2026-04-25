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
use std::future;
use std::io;
use std::io::Write as _;
use std::iter;
use std::sync::Arc;

use clap::ArgGroup;
use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::try_join_all;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::git;
use jj_lib::git::GitPushOptions;
use jj_lib::git::GitPushRefTargets;
use jj_lib::git::GitSettings;
use jj_lib::index::IndexResult;
use jj_lib::merge::Diff;
use jj_lib::op_store::RefTarget;
use jj_lib::operation::Operation;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::refs::LocalAndRemoteRef;
use jj_lib::refs::RefPushAction;
use jj_lib::refs::classify_ref_push_action;
use jj_lib::repo::Repo;
use jj_lib::revset::RemoteRefSymbolExpression;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetContainingFn;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetStreamExt as _;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::rewrite::CommitRewriter;
use jj_lib::signing::SignBehavior;
use jj_lib::str_util::StringExpression;
use jj_lib::view::View;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::has_tracked_remote_bookmarks;
use crate::cli_util::has_tracked_remote_tags;
use crate::cli_util::short_change_hash;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::cli_error_with_message;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::commands::git::get_single_remote;
use crate::complete;
use crate::formatter::Formatter;
use crate::git_util::GitSubprocessUi;
use crate::git_util::print_push_stats;
use crate::progress::ProgressWriter;
use crate::revset_util::parse_bookmark_name;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Push to a Git remote
///
/// By default, pushes tracking bookmarks pointing to
/// `remote_bookmarks(remote=<remote>)..@`. Use `--bookmark` to push specific
/// bookmarks. Use `--all` to push all bookmarks. Use `--change` to generate
/// bookmark names based on the change IDs of specific commits.
///
/// When pushing a bookmark, the command pushes all commits in the range from
/// the remote's current position up to and including the bookmark's target
/// commit. Any descendant commits beyond the bookmark are not pushed.
///
/// If the local bookmark has changed from the last fetch, push will update the
/// remote bookmark to the new position after passing safety checks. This is
/// similar to `git push --force-with-lease` - the remote is updated only if its
/// current state matches what Jujutsu last fetched.
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
///     https://docs.jj-vcs.dev/latest/bookmarks/#pushing-bookmarks-safety-checks
///
/// [bookmark conflicts]:
///     https://docs.jj-vcs.dev/latest/bookmarks/#conflicts

#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("specific").multiple(true)))]
#[command(group(ArgGroup::new("what").conflicts_with("specific")))]
pub struct GitPushArgs {
    /// The remote to push to (only named remotes are supported)
    ///
    /// This defaults to the `git.push` setting. If that is not configured, and
    /// if there are multiple remotes, the remote named "origin" will be used.
    #[arg(long)]
    #[arg(add = ArgValueCandidates::new(complete::git_remotes))]
    remote: Option<RemoteNameBuf>,

    /// Push only this bookmark, or bookmarks matching a pattern (can be
    /// repeated)
    ///
    /// If a bookmark isn't tracking anything yet, the remote bookmark will be
    /// tracked automatically.
    ///
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long, short, alias = "branch", group = "specific")]
    #[arg(add = ArgValueCandidates::new(complete::local_bookmarks))]
    bookmark: Vec<String>,

    /// Push only this tag, or tags matching a pattern (can be repeated)
    ///
    /// If a tag isn't tracking anything yet, the remote tag will be tracked
    /// automatically.
    ///
    /// By default, the specified pattern matches tag names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long, short, group = "specific")]
    #[arg(hide = true)] // TODO: unhide when this gets stabilized (#7528)
    tag: Vec<String>,

    /// Push all bookmarks (including new bookmarks)
    #[arg(long, group = "what")]
    all: bool,

    /// Push all tracked bookmarks
    ///
    /// This usually means that the bookmark was already pushed to or fetched
    /// from the [relevant remote].
    ///
    /// [relevant remote]:
    ///     https://docs.jj-vcs.dev/latest/bookmarks#remotes-and-tracked-bookmarks
    #[arg(long, group = "what")]
    tracked: bool,

    /// Push all deleted bookmarks
    ///
    /// Only tracked bookmarks can be successfully deleted on the remote. A
    /// warning will be printed if any untracked bookmarks on the remote
    /// correspond to missing local bookmarks.
    #[arg(long, conflicts_with = "specific")]
    deleted: bool,

    // TODO: Delete in jj 0.42.0+
    /// Allow pushing new bookmarks
    ///
    /// Newly-created remote bookmarks will be tracked automatically.
    #[arg(long, short = 'N', hide = true, conflicts_with = "what")]
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
        long = "revision",
        short,
        group = "specific",
        value_name = "REVSETS",
        alias = "revisions"
    )]
    // While `-r` will often be used with mutable revisions, immutable revisions
    // can be useful as parts of revsets or to push special-purpose branches.
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revisions: Vec<RevisionArg>,

    /// Push this commit by creating a bookmark (can be repeated)
    ///
    /// The created bookmark will be tracked automatically. Use the
    /// `templates.git_push_bookmark` setting to customize the generated
    /// bookmark name. The default is `"push-" ++ change_id.short()`.
    #[arg(long, short, group = "specific", value_name = "REVSETS")]
    // I'm guessing that `git push -c` is almost exclusively used with recently
    // created mutable revisions, even though it can in theory be used with
    // immutable ones as well. We can change it if the guess turns out to be
    // wrong.
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    change: Vec<RevisionArg>,

    /// Specify a new bookmark name and a revision to push under that name, e.g.
    /// '--named myfeature=@'
    ///
    /// Automatically tracks the bookmark if it is new.
    #[arg(long, group = "specific", value_name = "NAME=REVISION")]
    #[arg(add = ArgValueCompleter::new(complete::branch_name_equals_any_revision))]
    named: Vec<String>,

    /// Only display what will change on the remote
    #[arg(long)]
    dry_run: bool,

    /// Git push options
    #[arg(long, short)]
    option: Vec<String>,
}

fn make_updates_term(ref_updates: &GitPushRefTargets) -> String {
    let kind_updates = [
        ("bookmark", "bookmarks", &ref_updates.bookmarks),
        ("tag", "tags", &ref_updates.tags),
    ];
    kind_updates
        .into_iter()
        .filter_map(|(kind, kinds, refs)| match &**refs {
            [] => None,
            [(name, _)] => Some(format!("{kind} {}", name.as_symbol())),
            _ => Some(format!(
                "{kinds} {}",
                refs.iter().map(|(name, _)| name.as_symbol()).join(", ")
            )),
        })
        .join(", ")
}

const DEFAULT_REMOTE: &RemoteName = RemoteName::new("origin");

const TX_DESC_PUSH: &str = "push ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BookmarkMoveDirection {
    Forward,
    Backward,
    Sideways,
}

pub async fn cmd_git_push(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitPushArgs,
) -> Result<(), CommandError> {
    if args.allow_new {
        writeln!(
            ui.warning_default(),
            "--allow-new is deprecated, track bookmarks manually or configure \
             remotes.<name>.auto-track-bookmarks instead."
        )?;
    }
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
    let mut ref_updates = GitPushRefTargets::default();
    if args.all {
        let mut commits_validator =
            CommitsValidator::new(ui, tx.base_workspace_helper(), remote, args)?;
        for (name, targets) in view.local_remote_bookmarks(remote) {
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_new = true; // implied by --all
            match classify_bookmark_update(remote_symbol, targets, allow_new, args.deleted) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.bookmarks.push((name.to_owned(), update)),
                    Err(reason) => reason.print_bookmark(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        for (name, targets) in view.local_remote_tags(remote) {
            let remote_symbol = name.to_remote_symbol(remote);
            // TODO: push untracked tags when remote tags get stabilized (#7528)
            let allow_new = false;
            match classify_tag_update(remote_symbol, targets, allow_new, args.deleted) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.tags.push((name.to_owned(), update)),
                    Err(reason) => reason.print_tag(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!(
            "{TX_DESC_PUSH}all bookmarks/tags to git remote {remote}",
            remote = remote.as_symbol()
        );
    } else if args.tracked {
        let mut commits_validator =
            CommitsValidator::new(ui, tx.base_workspace_helper(), remote, args)?;
        for (name, targets) in view.local_remote_bookmarks(remote) {
            if !targets.remote_ref.is_tracked() {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_new = false; // doesn't matter
            match classify_bookmark_update(remote_symbol, targets, allow_new, args.deleted) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.bookmarks.push((name.to_owned(), update)),
                    Err(reason) => reason.print_bookmark(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        for (name, targets) in view.local_remote_tags(remote) {
            if !targets.remote_ref.is_tracked() {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_new = false; // doesn't matter
            match classify_tag_update(remote_symbol, targets, allow_new, args.deleted) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.tags.push((name.to_owned(), update)),
                    Err(reason) => reason.print_tag(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!(
            "{TX_DESC_PUSH}all tracked bookmarks/tags to git remote {remote}",
            remote = remote.as_symbol()
        );
    } else if args.deleted {
        // There shouldn't be new heads to push, but we run validation for consistency.
        let mut commits_validator =
            CommitsValidator::new(ui, tx.base_workspace_helper(), remote, args)?;
        for (name, targets) in view.local_remote_bookmarks(remote) {
            if targets.local_target.is_present() {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_new = false; // doesn't matter
            let allow_delete = true;
            match classify_bookmark_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.bookmarks.push((name.to_owned(), update)),
                    Err(reason) => reason.print_bookmark(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        for (name, targets) in view.local_remote_tags(remote) {
            if targets.local_target.is_present() {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_new = false; // doesn't matter
            let allow_delete = true;
            match classify_tag_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.tags.push((name.to_owned(), update)),
                    Err(reason) => reason.print_tag(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!(
            "{TX_DESC_PUSH}all deleted bookmarks/tags to git remote {remote}",
            remote = remote.as_symbol()
        );
    } else {
        let mut seen_bookmarks: HashSet<&RefName> = HashSet::new();
        let mut seen_tags: HashSet<&RefName> = HashSet::new();

        // --change and --named don't move existing bookmarks. If they did, be
        // careful to not select old state by -r/--revisions and bookmark names.
        let change_bookmark_names = create_change_bookmarks(ui, &mut tx, &args.change).await?;
        let named_bookmark_commits = try_join_all(args.named.iter().map(|arg| async {
            let (name, revision_arg) = parse_named_bookmark(arg)?;
            let commit = tx
                .base_workspace_helper()
                .resolve_single_rev(ui, &revision_arg)
                .await?;
            Ok::<_, CommandError>((name, commit))
        }))
        .await?;
        for (name, commit) in &named_bookmark_commits {
            ensure_new_bookmark_name(tx.repo(), name)?;
            tx.repo_mut()
                .set_local_bookmark_target(name, RefTarget::normal(commit.id().clone()));
        }
        let created_bookmarks = change_bookmark_names
            .iter()
            .chain(named_bookmark_commits.iter().map(|(name, _)| name))
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
                Ok(Some(update)) => ref_updates.bookmarks.push((name.to_owned(), update)),
                Ok(None) => writeln!(
                    ui.status(),
                    "Bookmark {remote_symbol} already matches {name}",
                    name = name.as_symbol()
                )?,
                Err(reason) => return Err(reason.into()),
            }
        }

        let view = tx.repo().view();
        // TODO: Delete in jj 0.42.0+
        let allow_new = args.allow_new || tx.settings().get("git.push-new-bookmarks")?;
        let bookmarks_by_name = find_bookmarks_to_push(ui, view, &args.bookmark, remote)?;
        for &(name, targets) in &bookmarks_by_name {
            if !seen_bookmarks.insert(name) {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            // Override allow_new if the bookmark is not tracked with any remote
            // already. The user has specified --bookmark, so their intent which
            // bookmarks to push is clear.
            let allow_new = allow_new || !has_tracked_remote_bookmarks(tx.repo(), name);
            let allow_delete = true; // named explicitly, allow delete without --delete
            match classify_bookmark_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => ref_updates.bookmarks.push((name.to_owned(), update)),
                Ok(None) => writeln!(
                    ui.status(),
                    "Bookmark {remote_symbol} already matches {name}",
                    name = name.as_symbol()
                )?,
                Err(reason) => return Err(reason.into()),
            }
        }

        let tags_by_name = find_tags_to_push(ui, view, &args.tag, remote)?;
        for &(name, targets) in &tags_by_name {
            if !seen_tags.insert(name) {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            // named explicitly, allow track or delete
            let allow_new = !has_tracked_remote_tags(tx.repo(), name);
            let allow_delete = true;
            match classify_tag_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => ref_updates.tags.push((name.to_owned(), update)),
                Ok(None) => writeln!(
                    ui.status(),
                    "Tag {remote_symbol} already matches {name}",
                    name = name.as_symbol()
                )?,
                Err(reason) => return Err(reason.into()),
            }
        }

        let mut commits_validator =
            CommitsValidator::new(ui, tx.base_workspace_helper(), remote, args)?;
        // Error out if explicitly-specified targets can't be pushed.
        commits_validator
            .validate_updates(&ref_updates)
            .await?
            .map_err(|reason| reason.to_command_error(tx.base_workspace_helper()))?;

        let use_default_revset = args.bookmark.is_empty()
            && args.tag.is_empty()
            && args.change.is_empty()
            && args.revisions.is_empty()
            && args.named.is_empty();
        let target_revisions = if use_default_revset {
            find_default_target_revisions(ui, tx.base_workspace_helper(), remote).await?
        } else {
            find_target_revisions(ui, tx.base_workspace_helper(), &args.revisions).await?
        };
        for (name, targets) in tx.base_repo().view().local_remote_bookmarks(remote) {
            if !matches_local_target(targets, &target_revisions) || !seen_bookmarks.insert(name) {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_delete = false;
            match classify_bookmark_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.bookmarks.push((name.to_owned(), update)),
                    Err(reason) => reason.print_bookmark(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        for (name, targets) in tx.base_repo().view().local_remote_tags(remote) {
            if !matches_local_target(targets, &target_revisions) || !seen_tags.insert(name) {
                continue;
            }
            let remote_symbol = name.to_remote_symbol(remote);
            let allow_delete = false;
            match classify_tag_update(remote_symbol, targets, allow_new, allow_delete) {
                Ok(Some(update)) => match commits_validator.validate_update(&update).await? {
                    Ok(()) => ref_updates.tags.push((name.to_owned(), update)),
                    Err(reason) => reason.print_tag(ui, tx.base_workspace_helper(), name)?,
                },
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }

        tx_description = format!(
            "{TX_DESC_PUSH}{names} to git remote {remote}",
            names = make_updates_term(&ref_updates),
            remote = remote.as_symbol()
        );
    }
    if ref_updates.bookmarks.is_empty() && ref_updates.tags.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    if !args.dry_run && tx.settings().get_bool("git.sign-on-push")? {
        let to_push_expr = ready_to_push_revset_expression(&tx, remote, &ref_updates);
        ref_updates = sign_commits_before_push(ui, &mut tx, to_push_expr, ref_updates).await?;
    }

    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "Changes to push to {remote}:",
            remote = remote.as_symbol()
        )?;
        print_commits_ready_to_push(formatter.as_mut(), tx.repo(), &ref_updates)?;
    }

    if args.dry_run {
        writeln!(ui.status(), "Dry-run requested, not pushing.")?;
        return Ok(());
    }

    let git_settings = GitSettings::from_settings(tx.settings())?;
    let options = GitPushOptions {
        remote_push_options: args.option.clone(),
    };
    let push_stats = git::push_refs(
        tx.repo_mut(),
        git_settings.to_subprocess_options(),
        remote,
        &ref_updates,
        &mut GitSubprocessUi::new(ui),
        &options,
    )?;
    print_push_stats(ui, &push_stats)?;
    // TODO: On partial success, locally-created --change/--named bookmarks will
    // be committed. It's probably better to remove failed local bookmarks.
    if push_stats.all_ok() || push_stats.some_exported() {
        tx.finish(ui, tx_description).await?;
    }
    if push_stats.all_ok() {
        Ok(())
    } else {
        Err(user_error("Failed to push some bookmarks"))
    }
}

#[derive(Clone, Debug)]
struct RejectedCommitReason {
    commit: Commit,
    message: String,
    hint: Option<String>,
}

impl RejectedCommitReason {
    fn print_bookmark(
        &self,
        ui: &Ui,
        workspace_helper: &WorkspaceCommandHelper,
        name: &RefName,
    ) -> io::Result<()> {
        self.print_inner(ui, workspace_helper, "bookmark", name)
    }

    fn print_tag(
        &self,
        ui: &Ui,
        workspace_helper: &WorkspaceCommandHelper,
        name: &RefName,
    ) -> io::Result<()> {
        self.print_inner(ui, workspace_helper, "tag", name)
    }

    fn print_inner(
        &self,
        ui: &Ui,
        workspace_helper: &WorkspaceCommandHelper,
        kind: &str,
        name: &RefName,
    ) -> io::Result<()> {
        writeln!(
            ui.warning_default(),
            "Won't push {kind} {name}: commit {id} {message}",
            name = name.as_symbol(),
            id = short_commit_hash(self.commit.id()),
            message = self.message,
        )?;
        if let Some(mut formatter) = ui.status_formatter() {
            write!(formatter, "  ")?;
            workspace_helper.write_commit_summary(formatter.as_mut(), &self.commit)?;
            writeln!(formatter)?;
        }
        if let Some(hint) = &self.hint {
            writeln!(ui.hint_default(), "{hint}")?;
        }
        Ok(())
    }

    fn to_command_error(&self, workspace_helper: &WorkspaceCommandHelper) -> CommandError {
        let mut error = user_error(format!(
            "Won't push commit {id} since it {message}",
            id = short_commit_hash(self.commit.id()),
            message = self.message,
        ));
        error.add_formatted_hint_with(|formatter| {
            write!(formatter, "Rejected commit: ")?;
            workspace_helper.write_commit_summary(formatter, &self.commit)?;
            Ok(())
        });
        error.extend_hints(self.hint.clone());
        error
    }
}

/// Validates that the commits that will be pushed are ready (have authorship
/// information, are not conflicted, etc.).
struct CommitsValidator<'repo> {
    repo: &'repo dyn Repo,
    known_heads: Vec<CommitId>,
    immutable_heads: Arc<ResolvedRevsetExpression>,
    private_commits: Option<(String, Box<RevsetContainingFn<'repo>>)>,
    allow_empty_description: bool,
}

impl<'repo> CommitsValidator<'repo> {
    fn new(
        ui: &Ui,
        workspace_helper: &'repo WorkspaceCommandHelper,
        remote: &RemoteName,
        args: &GitPushArgs,
    ) -> Result<Self, CommandError> {
        let repo = workspace_helper.repo().as_ref();
        let known_heads = repo
            .view()
            .remote_bookmarks(remote)
            .flat_map(|(_, old_head)| old_head.target.added_ids())
            .cloned()
            .collect();
        let immutable_heads = workspace_helper
            .attach_revset_evaluator(workspace_helper.env().immutable_heads_expression().clone())
            .resolve()?;
        let private_commits = if !args.allow_private {
            let settings = workspace_helper.settings();
            let revset_str = settings.get_string("git.private-commits")?;
            let is_private = workspace_helper
                .parse_revset(ui, &RevisionArg::from(revset_str.clone()))?
                .evaluate()?
                .containing_fn();
            Some((revset_str, is_private))
        } else {
            None
        };
        Ok(Self {
            repo,
            known_heads,
            immutable_heads,
            private_commits,
            allow_empty_description: args.allow_empty_description,
        })
    }

    async fn validate_update(
        &mut self,
        update: &Diff<Option<CommitId>>,
    ) -> Result<Result<(), RejectedCommitReason>, RevsetEvaluationError> {
        self.validate_commits(update.after.as_slice()).await
    }

    async fn validate_updates(
        &mut self,
        updates: &GitPushRefTargets,
    ) -> Result<Result<(), RejectedCommitReason>, RevsetEvaluationError> {
        let new_heads = itertools::chain(&updates.bookmarks, &updates.tags)
            .filter_map(|(_, update)| update.after.clone())
            .collect_vec();
        self.validate_commits(&new_heads).await
    }

    async fn validate_commits(
        &mut self,
        new_heads: &[CommitId],
    ) -> Result<Result<(), RejectedCommitReason>, RevsetEvaluationError> {
        let new_commits = RevsetExpression::commits(self.known_heads.clone())
            .union(&self.immutable_heads)
            .range(&RevsetExpression::commits(new_heads.to_vec()));
        let mut commit_stream = new_commits
            .evaluate(self.repo)?
            .stream()
            .commits(self.repo.store());
        while let Some(commit) = commit_stream.try_next().await? {
            let mut reasons = vec![];
            let mut hint = None;
            if commit.description().is_empty() && !self.allow_empty_description {
                reasons.push("has no description");
            }
            if commit.author().name.is_empty()
                || commit.author().email.is_empty()
                || commit.committer().name.is_empty()
                || commit.committer().email.is_empty()
            {
                reasons.push("has no author and/or committer set");
            }
            if commit.has_conflict() {
                reasons.push("has conflicts");
            }
            if let Some((revset_str, is_private)) = &self.private_commits
                && is_private(commit.id())?
            {
                reasons.push("is private");
                hint = Some(format!("Configured git.private-commits: '{revset_str}'"));
            }
            if reasons.is_empty() {
                continue;
            }
            return Ok(Err(RejectedCommitReason {
                commit,
                message: reasons.join(" and "),
                hint,
            }));
        }

        // No need to validate ancestors again
        self.known_heads.extend(new_heads.iter().cloned());
        Ok(Ok(()))
    }
}

fn ready_to_push_revset_expression(
    tx: &WorkspaceCommandTransaction,
    remote: &RemoteName,
    ref_updates: &GitPushRefTargets,
) -> Arc<UserRevsetExpression> {
    let workspace_helper = tx.base_workspace_helper();
    let repo = workspace_helper.repo();
    let new_heads = itertools::chain(&ref_updates.bookmarks, &ref_updates.tags)
        .filter_map(|(_, update)| update.after.clone())
        .collect_vec();
    let old_heads = repo
        .view()
        .remote_bookmarks(remote)
        .flat_map(|(_, old_head)| old_head.target.added_ids())
        .cloned()
        .collect_vec();
    RevsetExpression::commits(old_heads)
        .union(workspace_helper.env().immutable_heads_expression())
        .range(&RevsetExpression::commits(new_heads))
}

/// Signs commits before pushing.
///
/// Returns the updated list of bookmark names and corresponding
/// [`BookmarkPushUpdate`]s.
async fn sign_commits_before_push(
    ui: &Ui,
    tx: &mut WorkspaceCommandTransaction<'_>,
    commits_to_push: Arc<UserRevsetExpression>,
    ref_updates: GitPushRefTargets,
) -> Result<GitPushRefTargets, CommandError> {
    let mut sign_settings = tx.settings().sign_settings();
    sign_settings.behavior = SignBehavior::Own;
    let commit_ids: IndexSet<CommitId> = tx
        .base_workspace_helper()
        .attach_revset_evaluator(commits_to_push)
        .evaluate_to_commits()?
        // TODO: make filter condition configurable by revset?
        .try_filter(|commit| {
            future::ready(!commit.is_signed() && sign_settings.should_sign(commit.store_commit()))
        })
        .map_ok(|commit| commit.id().clone())
        .try_collect()
        .await?;
    if commit_ids.is_empty() {
        return Ok(ref_updates);
    }

    let mut old_to_new_commits_map: HashMap<CommitId, CommitId> = HashMap::new();
    let mut num_rebased_descendants = 0;
    {
        let mut progress_writer = ProgressWriter::new(ui, "Signing");

        tx.repo_mut()
            .transform_descendants(
                commit_ids.iter().cloned().collect_vec(),
                async |rewriter: CommitRewriter<'_>| {
                    let old_commit = rewriter.old_commit();
                    let old_commit_id = old_commit.id().clone();
                    if let Some(writer) = &mut progress_writer {
                        writer
                            .display(&short_change_hash(old_commit.change_id()))
                            .ok();
                    }
                    if commit_ids.contains(&old_commit_id) {
                        let commit = rewriter
                            .reparent()
                            .set_sign_behavior(sign_settings.behavior)
                            .write()
                            .await?;
                        old_to_new_commits_map.insert(old_commit_id, commit.id().clone());
                    } else {
                        num_rebased_descendants += 1;
                        let commit = rewriter.reparent().write().await?;
                        old_to_new_commits_map.insert(old_commit_id, commit.id().clone());
                    }
                    Ok(())
                },
            )
            .await?;
    }

    let map_to_new_commits = |updates: Vec<(RefNameBuf, Diff<Option<CommitId>>)>| {
        updates
            .into_iter()
            .map(|(name, Diff { before, after })| {
                let after = after.map(|id| old_to_new_commits_map.get(&id).cloned().unwrap_or(id));
                (name, Diff { before, after })
            })
            .collect()
    };
    let ref_updates = GitPushRefTargets {
        bookmarks: map_to_new_commits(ref_updates.bookmarks),
        tags: map_to_new_commits(ref_updates.tags),
    };

    if let Some(mut formatter) = ui.status_formatter() {
        let num_updated_signatures = commit_ids.len();
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

    Ok(ref_updates)
}

fn print_commits_ready_to_push(
    formatter: &mut dyn Formatter,
    repo: &dyn Repo,
    ref_updates: &GitPushRefTargets,
) -> Result<(), CommandError> {
    let to_direction =
        |old_target: &CommitId, new_target: &CommitId| -> IndexResult<BookmarkMoveDirection> {
            assert_ne!(old_target, new_target);
            if repo.index().is_ancestor(old_target, new_target)? {
                Ok(BookmarkMoveDirection::Forward)
            } else if repo.index().is_ancestor(new_target, old_target)? {
                Ok(BookmarkMoveDirection::Backward)
            } else {
                Ok(BookmarkMoveDirection::Sideways)
            }
        };
    let describe_update = |update: &Diff<Option<CommitId>>| -> IndexResult<String> {
        let desc = match (&update.before, &update.after) {
            (Some(old_target), Some(new_target)) => {
                let old = short_commit_hash(old_target);
                let new = short_commit_hash(new_target);
                // TODO: People on Discord suggest "... forward by n commits",
                // possibly "... sideways (X forward, Y back)".
                match to_direction(old_target, new_target)? {
                    BookmarkMoveDirection::Forward => {
                        format!("move forward from {old} to {new}")
                    }
                    BookmarkMoveDirection::Backward => {
                        format!("move backward from {old} to {new}")
                    }
                    BookmarkMoveDirection::Sideways => {
                        format!("move sideways from {old} to {new}")
                    }
                }
            }
            (Some(old_target), None) => {
                format!("delete from {old}", old = short_commit_hash(old_target))
            }
            (None, Some(new_target)) => {
                format!("add to {new}", new = short_commit_hash(new_target))
            }
            (None, None) => {
                panic!("Not pushing any change");
            }
        };
        Ok(desc)
    };

    // TODO: Add color
    let kind_updates = [
        ("bookmark", &ref_updates.bookmarks),
        ("tag", &ref_updates.tags),
    ];
    for (kind, updates) in kind_updates {
        for (name, update) in updates {
            let desc = describe_update(update)?;
            writeln!(
                formatter,
                "  {kind}: {name} [{desc}]",
                name = name.as_symbol()
            )?;
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
struct RejectedRefUpdateReason {
    message: String,
    hint: Option<String>,
}

impl RejectedRefUpdateReason {
    fn print(&self, ui: &Ui) -> io::Result<()> {
        writeln!(ui.warning_default(), "{}", self.message)?;
        if let Some(hint) = &self.hint {
            writeln!(ui.hint_default(), "{hint}")?;
        }
        Ok(())
    }
}

impl From<RejectedRefUpdateReason> for CommandError {
    fn from(reason: RejectedRefUpdateReason) -> Self {
        let RejectedRefUpdateReason { message, hint } = reason;
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
) -> Result<Option<Diff<Option<CommitId>>>, RejectedRefUpdateReason> {
    let push_action = classify_ref_push_action(targets);
    match push_action {
        RefPushAction::AlreadyMatches => Ok(None),
        RefPushAction::LocalConflicted => Err(RejectedRefUpdateReason {
            message: format!(
                "Bookmark {name} is conflicted",
                name = remote_symbol.name.as_symbol()
            ),
            hint: Some(
                "Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up."
                    .to_owned(),
            ),
        }),
        RefPushAction::RemoteConflicted => Err(RejectedRefUpdateReason {
            message: format!("Bookmark {remote_symbol} is conflicted"),
            hint: Some("Run `jj git fetch` to update the conflicted remote bookmark.".to_owned()),
        }),
        RefPushAction::RemoteUntracked => Err(RejectedRefUpdateReason {
            message: format!("Non-tracking remote bookmark {remote_symbol} exists"),
            hint: Some(format!(
                "Run `jj bookmark track {name} --remote={remote}` to import the remote bookmark.",
                name = remote_symbol.name.as_symbol(),
                remote = remote_symbol.remote.as_symbol()
            )),
        }),
        // TODO: deprecate --allow-new and make classify_bookmark_push_action()
        // reject untracked remote?
        RefPushAction::Update(_) if !targets.remote_ref.is_tracked() && !allow_new => {
            Err(RejectedRefUpdateReason {
                message: format!("Refusing to create new remote bookmark {remote_symbol}"),
                hint: Some(format!(
                    "Run `jj bookmark track {name} --remote={remote}` and try again.",
                    name = remote_symbol.name.as_symbol(),
                    remote = remote_symbol.remote.as_symbol()
                )),
            })
        }
        RefPushAction::Update(update) if update.after.is_none() && !allow_delete => {
            Err(RejectedRefUpdateReason {
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
        RefPushAction::Update(update) => Ok(Some(update)),
    }
}

fn classify_tag_update(
    remote_symbol: RemoteRefSymbol<'_>,
    targets: LocalAndRemoteRef<'_>,
    allow_new: bool,
    allow_delete: bool,
) -> Result<Option<Diff<Option<CommitId>>>, RejectedRefUpdateReason> {
    let push_action = classify_ref_push_action(targets);
    match push_action {
        RefPushAction::AlreadyMatches => Ok(None),
        RefPushAction::LocalConflicted => Err(RejectedRefUpdateReason {
            message: format!(
                "Tag {name} is conflicted",
                name = remote_symbol.name.as_symbol()
            ),
            hint: Some(
                "Run `jj tag list` to inspect, and use `jj tag set` to fix it up.".to_owned(),
            ),
        }),
        RefPushAction::RemoteConflicted => Err(RejectedRefUpdateReason {
            message: format!("Tag {remote_symbol} is conflicted"),
            hint: Some("Run `jj git fetch` to update the conflicted remote tag.".to_owned()),
        }),
        RefPushAction::RemoteUntracked => Err(RejectedRefUpdateReason {
            message: format!("Non-tracking remote tag {remote_symbol} exists"),
            // No suggestion because tags are tracked by default
            hint: None,
        }),
        RefPushAction::Update(_) if !targets.remote_ref.is_tracked() && !allow_new => {
            Err(RejectedRefUpdateReason {
                message: format!("Refusing to create new remote tag {remote_symbol}"),
                // TODO: suggest `jj tag track`?
                hint: None,
            })
        }
        RefPushAction::Update(update) if update.after.is_none() && !allow_delete => {
            Err(RejectedRefUpdateReason {
                message: format!(
                    "Refusing to push deleted tag {name}",
                    name = remote_symbol.name.as_symbol(),
                ),
                // TODO: suggest `jj tag forget`?
                hint: Some("Push deleted tags with --deleted.".to_owned()),
            })
        }
        RefPushAction::Update(update) => Ok(Some(update)),
    }
}

fn ensure_new_bookmark_name(repo: &dyn Repo, name: &RefName) -> Result<(), CommandError> {
    let symbol = name.as_symbol();
    if repo.view().get_local_bookmark(name).is_present() {
        return Err(
            user_error(format!("Bookmark already exists: {symbol}")).hinted(format!(
                "Use 'jj bookmark move' to move it, and 'jj git push -b {symbol}' to push it"
            )),
        );
    }
    if has_tracked_remote_bookmarks(repo, name) {
        return Err(user_error(format!(
            "Tracked remote bookmarks exist for deleted bookmark: {symbol}"
        ))
        .hinted(format!(
            "Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark untrack \
             {symbol}` to disassociate them."
        )));
    }
    Ok(())
}

fn parse_named_bookmark(name_revision: &str) -> Result<(RefNameBuf, RevisionArg), CommandError> {
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
    Ok((name, RevisionArg::from(revision_str.to_owned())))
}

/// Creates bookmarks based on the change IDs.
async fn create_change_bookmarks(
    ui: &Ui,
    tx: &mut WorkspaceCommandTransaction<'_>,
    changes: &[RevisionArg],
) -> Result<Vec<RefNameBuf>, CommandError> {
    if changes.is_empty() {
        // NOTE: we don't want resolve_some_revsets_default_single to fail if the
        // changes argument wasn't provided, so handle that
        return Ok(vec![]);
    }

    let all_commits: Vec<_> = tx
        .base_workspace_helper()
        .resolve_some_revsets(ui, changes)
        .await?
        .iter()
        .map(|id| tx.repo().store().get_commit(id))
        .try_collect()?;
    let bookmark_names: Vec<_> = {
        let template_text = tx.settings().get_string("templates.git_push_bookmark")?;
        let template = tx.parse_commit_template(ui, &template_text)?;
        all_commits
            .iter()
            .map(|commit| {
                let output = template.format_plain_text(commit);
                let name = String::from_utf8(output).map_err(|err| {
                    user_error_with_message("Invalid character in bookmark name", err.utf8_error())
                })?;
                if name.is_empty() {
                    return Err(user_error("Empty bookmark name generated"));
                }
                Ok(RefNameBuf::from(name))
            })
            .try_collect()?
    };

    for (commit, name) in iter::zip(&all_commits, &bookmark_names) {
        let target = RefTarget::normal(commit.id().clone());
        if tx.repo().view().get_local_bookmark(name) == &target {
            // Existing bookmark pointing to the commit, which is allowed
            continue;
        }
        ensure_new_bookmark_name(tx.repo(), name)?;
        writeln!(
            ui.status(),
            "Creating bookmark {name} for revision {change_id:.12}",
            name = name.as_symbol(),
            change_id = commit.change_id()
        )?;
        tx.repo_mut().set_local_bookmark_target(name, target);
    }
    Ok(bookmark_names)
}

fn find_bookmarks_to_push<'a>(
    ui: &Ui,
    view: &'a View,
    bookmark_patterns: &[String],
    remote: &RemoteName,
) -> Result<Vec<(&'a RefName, LocalAndRemoteRef<'a>)>, CommandError> {
    let bookmark_expr = parse_union_name_patterns(ui, bookmark_patterns)?;
    let bookmark_matcher = bookmark_expr.to_matcher();
    let matching_bookmarks = view
        .local_remote_bookmarks_matching(&bookmark_matcher, remote)
        .filter(|(_, targets)| {
            // If the remote exists but is not tracked, the absent local shouldn't
            // be considered a deleted bookmark.
            targets.local_target.is_present() || targets.remote_ref.is_tracked()
        })
        .collect();
    let mut unmatched_names = bookmark_expr
        .exact_strings()
        .map(RefName::new)
        .filter(|&name| {
            let symbol = name.to_remote_symbol(remote);
            view.get_local_bookmark(name).is_absent()
                && !view.get_remote_bookmark(symbol).is_tracked()
        })
        .peekable();
    if unmatched_names.peek().is_some() {
        writeln!(
            ui.warning_default(),
            "No matching bookmarks for names: {}",
            unmatched_names.map(|name| name.as_symbol()).join(", ")
        )?;
    }
    Ok(matching_bookmarks)
}

fn find_tags_to_push<'a>(
    ui: &Ui,
    view: &'a View,
    tag_patterns: &[String],
    remote: &RemoteName,
) -> Result<Vec<(&'a RefName, LocalAndRemoteRef<'a>)>, CommandError> {
    let tag_expr = parse_union_name_patterns(ui, tag_patterns)?;
    let tag_matcher = tag_expr.to_matcher();
    let matching_tags = view
        .local_remote_tags_matching(&tag_matcher, remote)
        .filter(|(_, targets)| {
            // If the remote exists but is not tracked, the absent local shouldn't
            // be considered a deleted tag.
            targets.local_target.is_present() || targets.remote_ref.is_tracked()
        })
        .collect();
    let mut unmatched_names = tag_expr
        .exact_strings()
        .map(RefName::new)
        .filter(|&name| {
            let symbol = name.to_remote_symbol(remote);
            view.get_local_tag(name).is_absent() && !view.get_remote_tag(symbol).is_tracked()
        })
        .peekable();
    if unmatched_names.peek().is_some() {
        writeln!(
            ui.warning_default(),
            "No matching tags for names: {}",
            unmatched_names.map(|name| name.as_symbol()).join(", ")
        )?;
    }
    Ok(matching_tags)
}

async fn find_default_target_revisions(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    remote: &RemoteName,
) -> Result<HashSet<CommitId>, CommandError> {
    // remote_bookmarks(remote=<remote>)..@
    let workspace_name = workspace_command.workspace_name();
    let expression = RevsetExpression::remote_bookmarks(
        RemoteRefSymbolExpression {
            name: StringExpression::all(),
            remote: StringExpression::exact(remote),
        },
        None,
    )
    .range(&RevsetExpression::working_copy(workspace_name.to_owned()))
    .intersection(
        &RevsetExpression::bookmarks(StringExpression::all())
            .union(&RevsetExpression::tags(StringExpression::all())),
    );
    let commit_ids = workspace_command
        .attach_revset_evaluator(expression)
        .evaluate_to_commit_ids()?
        .peekable();
    let mut commit_ids = std::pin::pin!(commit_ids);
    if commit_ids.as_mut().peek().await.is_none() {
        writeln!(
            ui.warning_default(),
            "No bookmarks/tags found in the default push revset: \
             remote_bookmarks(remote={remote})..@",
            remote = remote.as_symbol()
        )?;
    }
    Ok(commit_ids.try_collect().await?)
}

async fn find_target_revisions(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    revisions: &[RevisionArg],
) -> Result<HashSet<CommitId>, CommandError> {
    let mut revision_commit_ids = HashSet::new();
    for rev_arg in revisions {
        let mut expression = workspace_command.parse_revset(ui, rev_arg)?;
        expression.intersect_with(
            &RevsetExpression::bookmarks(StringExpression::all())
                .union(&RevsetExpression::tags(StringExpression::all())),
        );
        let commit_ids = expression.evaluate_to_commit_ids()?.peekable();
        let mut commit_ids = std::pin::pin!(commit_ids);
        if commit_ids.as_mut().as_mut().peek().await.is_none() {
            writeln!(
                ui.warning_default(),
                "No bookmarks/tags point to the specified revisions: {rev_arg}"
            )?;
        }
        while let Some(commit_id) = commit_ids.try_next().await? {
            revision_commit_ids.insert(commit_id);
        }
    }
    Ok(revision_commit_ids)
}

fn matches_local_target(targets: LocalAndRemoteRef<'_>, revisions: &HashSet<CommitId>) -> bool {
    let mut local_ids = targets.local_target.added_ids();
    local_ids.any(|id| revisions.contains(id))
}

pub fn is_push_operation(op: &Operation) -> bool {
    op.metadata().description.starts_with(TX_DESC_PUSH)
}
