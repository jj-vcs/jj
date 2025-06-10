// Copyright 2020-2025 The Jujutsu Authors
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

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbolBuf;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::compute_move_commits;
use jj_lib::rewrite::MoveCommitsLocation;
use jj_lib::rewrite::MoveCommitsTarget;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::str_util::StringPattern;

use crate::cli_util::CommandHelper;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::commands::git::fetch::do_git_fetch;
use crate::commands::git::fetch::get_default_fetch_remotes;
use crate::commands::git::resolve_remote_patterns;
use crate::complete;
use crate::ui::Ui;

/// Fetch from remotes and rebase local changes
///
/// This command fetches from Git remotes and rebases local commits that were
/// descendants of remote-tracking bookmarks onto the new remote heads. This
/// provides a workflow similar to `git pull --rebase` but operates on all
/// tracked remote bookmarks simultaneously.
///
/// The rebase operation automatically drops any local commits that have been
/// merged upstream.
#[derive(clap::Args, Clone, Debug)]
pub struct GitSyncArgs {
    /// The remotes to sync with
    ///
    /// This defaults to the `git.fetch` setting. If that is not configured, and
    /// if there are multiple remotes, the remote named "origin" will be used.
    ///
    /// By default, the specified remote names match exactly. Use a [string
    /// pattern], e.g. `--remote 'glob:*'`, to select remotes using
    /// patterns.
    ///
    /// [string pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets#string-patterns
    #[arg(
        long = "remote",
        short = 'r',
        value_name = "REMOTE",
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::git_remotes),
    )]
    remotes: Vec<StringPattern>,

    /// Sync only these bookmarks, or bookmarks matching a pattern
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// expand `*` as a glob, e.g. `--branch 'glob:push-*'`. Other wildcard
    /// characters such as `?` are *not* supported.
    #[arg(
        long = "bookmark",
        short = 'b',
        alias = "branch",
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::bookmarks),
    )]
    bookmarks: Vec<StringPattern>,

    /// Sync with all remotes
    #[arg(long, conflicts_with = "remotes")]
    all_remotes: bool,
}

#[tracing::instrument(skip_all)]
pub fn cmd_git_sync(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitSyncArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    // Determine which remotes to sync
    let remote_patterns = if args.all_remotes {
        vec![StringPattern::everything()]
    } else if args.remotes.is_empty() {
        get_default_fetch_remotes(ui, &workspace_command)?
    } else {
        args.remotes.clone()
    };

    let resolved_remotes =
        resolve_remote_patterns(ui, workspace_command.repo().store(), &remote_patterns)?;
    let remotes = resolved_remotes.iter().map(|r| r.as_ref()).collect_vec();

    let mut tx = workspace_command.start_transaction();

    // Log initial state of all local bookmarks
    tracing::debug!("Git sync starting - logging initial bookmark state");
    for (name, target) in tx.repo().view().local_bookmarks() {
        tracing::debug!(?name, ?target, "Before sync - local bookmark");
    }

    // Capture the pre-fetch state of remote tracking bookmarks
    let mut pre_fetch_heads: HashMap<RemoteRefSymbolBuf, CommitId> = HashMap::new();
    let mut used_fallback: HashSet<RemoteRefSymbolBuf> = HashSet::new();

    for remote in &remotes {
        for (name, local_remote_ref) in tx.repo().view().local_remote_bookmarks(remote) {
            // We only process tracked bookmarks as we're syncing with remotes
            if local_remote_ref.remote_ref.is_tracked() {
                // Use remote_ref.target (actual remote position) not local_target (user's local moves)
                // Example: origin@origin -> A (base), origin -> B (user moved it locally)

                if let Some(commit_id) = local_remote_ref.remote_ref.target.as_normal() {
                    // Check if the commit is visible (not hidden)
                    // Hidden commits can occur after force-pushes or history rewrites
                    match tx.repo().store().get_commit(commit_id) {
                        Ok(_) => {
                            // Commit exists and is visible - use it as the pre-fetch head
                            let symbol = name.to_remote_symbol(remote).to_owned();
                            pre_fetch_heads.insert(symbol.clone(), commit_id.clone());
                            tracing::debug!(
                                ?name,
                                ?commit_id,
                                ?symbol,
                                "Using remote bookmark target as pre-fetch head"
                            );
                        }
                        Err(BackendError::ObjectNotFound { .. }) => {
                            // The remote bookmark points to a hidden/missing commit.
                            // This can happen after a force-push on the remote.
                            if let Some(local_id) = local_remote_ref.local_target.as_normal() {
                                let symbol = name.to_remote_symbol(remote).to_owned();
                                pre_fetch_heads.insert(symbol.clone(), local_id.clone());
                                used_fallback.insert(symbol);
                                tracing::debug!(
                                    ?name,
                                    ?commit_id,
                                    ?local_id,
                                    "Remote bookmark points to hidden commit, using local target \
                                     as fallback"
                                );
                            }
                        }
                        Err(err) => {
                            // Other backend errors should be propagated
                            return Err(err.into());
                        }
                    }
                }
            }
        }
    }

    let fetch_branches = vec![StringPattern::everything()];
    do_git_fetch(ui, &mut tx, &remotes, &fetch_branches)?;

    // Identify what needs to be rebased
    let mut rebase_operations: Vec<(RemoteRefSymbolBuf, CommitId, CommitId)> = Vec::new();

    for (symbol, old_head_id) in &pre_fetch_heads {
        // Look up the new head for this symbol
        let new_remote_ref = tx.repo().view().get_remote_bookmark(symbol.as_ref());

        if let Some(new_head_id) = new_remote_ref.target.as_normal() {
            if new_head_id != old_head_id {
                // Apply branch filtering if specified
                if !args.bookmarks.is_empty() {
                    let matches_filter = args
                        .bookmarks
                        .iter()
                        .any(|pattern| pattern.matches(symbol.name.as_str()));
                    if !matches_filter {
                        continue;
                    }
                }

                rebase_operations.push((
                    symbol.clone(),
                    old_head_id.clone(),
                    new_head_id.clone(),
                ));
            }
        }
    }

    // Execute the rebases
    let mut num_rebased_stacks = 0;
    let mut total_rebased_commits = 0;
    let mut total_abandoned_commits = 0;

    for (symbol, old_head_id, new_head_id) in rebase_operations {
        let local_bookmark = tx.repo().view().get_local_bookmark(&symbol.name);

        let branch_commit_ids: Vec<CommitId> = local_bookmark.added_ids().cloned().collect();

        if branch_commit_ids.is_empty() {
            writeln!(
                ui.status(),
                "No local bookmark '{}' exists (remote '{}' is tracked but has no local counterpart)",
                symbol.name.as_str(),
                symbol
            )?;
            continue;
        }

        let used_fallback_for_this = used_fallback.contains(&symbol);
        
        let needs_rebase = if used_fallback_for_this {
            // When we used fallback (after force-push), check if any local commits
            // are NOT already ancestors of the new head (meaning they need rebasing)
            !branch_commit_ids.iter().all(|id| {
                tx.repo().index().is_ancestor(&new_head_id, id)
            })
        } else {
            branch_commit_ids.iter().any(|id| {
                let is_descendant_of_old = tx.repo().index().is_ancestor(&old_head_id, id);
                let is_descendant_of_new = tx.repo().index().is_ancestor(&new_head_id, id);
                is_descendant_of_old && !is_descendant_of_new
            })
        };

        if !needs_rebase {
            writeln!(ui.status(), "Bookmark '{}' is already up to date", symbol.name.as_str())?;
            continue;
        }

        writeln!(
            ui.status(),
            "Rebasing bookmark '{}' onto {} (from {})",
            symbol.name.as_str(),
            &new_head_id.hex()[..12],
            symbol
        )?;

        let root_commit_ids = crate::commands::rebase::find_branch_fork_point_roots(
            tx.repo(),
            &[new_head_id.clone()],
            &branch_commit_ids,
        ).map_err(|err| user_error(format!("Revset evaluation failed: {err}")))?;

        tracing::debug!(
            bookmark_name = ?symbol.name,
            ?root_commit_ids,
            ?new_head_id,
            "Rebasing bookmark using fork-point roots"
        );

        let move_location = MoveCommitsLocation {
            new_parent_ids: vec![new_head_id.clone()],
            new_child_ids: vec![],
            target: MoveCommitsTarget::Roots(root_commit_ids),
        };

        let rebase_options = RebaseOptions {
            empty: jj_lib::rewrite::EmptyBehaviour::AbandonAllEmpty,
            ..Default::default()
        };

        let computed_move = compute_move_commits(tx.repo(), &move_location)?;
        let stats = computed_move.apply(tx.repo_mut(), &rebase_options)?;

        let rebased_count = stats.num_rebased_targets + stats.num_rebased_descendants;
        if rebased_count > 0 || stats.num_abandoned_empty > 0 {
            writeln!(
                ui.status(),
                "  Rebased {} commits for bookmark '{}' ({} abandoned as already merged)",
                rebased_count,
                symbol.name.as_str(),
                stats.num_abandoned_empty
            )?;
        }

        total_rebased_commits += rebased_count;
        total_abandoned_commits += stats.num_abandoned_empty;
        num_rebased_stacks += 1;
    }

    // Finish the transaction
    let tx_description = if num_rebased_stacks > 0 {
        format!(
            "git sync: fetched and rebased {} commits across {} bookmark updates from {}",
            total_rebased_commits,
            num_rebased_stacks,
            remotes.iter().map(|n| n.as_symbol()).join(", ")
        )
    } else {
        format!(
            "git sync: fetched from {} (no local changes to rebase)",
            remotes.iter().map(|n| n.as_symbol()).join(", ")
        )
    };

    tracing::debug!("Git sync complete - logging final bookmark state");
    for (name, target) in tx.repo().view().local_bookmarks() {
        tracing::debug!(?name, ?target, "After sync - local bookmark");
    }

    tx.finish(ui, tx_description)?;

    // Summary message
    if num_rebased_stacks > 0 {
        if total_abandoned_commits > 0 {
            writeln!(
                ui.status(),
                "Synced and rebased {total_rebased_commits} commits ({total_abandoned_commits} \
                 already merged) across {num_rebased_stacks} bookmark updates."
            )?;
        } else {
            writeln!(
                ui.status(),
                "Synced and rebased {total_rebased_commits} commits across {num_rebased_stacks} \
                 bookmark updates."
            )?;
        }
    } else {
        writeln!(ui.status(), "No local changes to sync.")?;
    }

    Ok(())
}
