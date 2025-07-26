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

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::RemoteRefSymbolBuf;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
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

    for remote in &remotes {
        for (name, local_remote_ref) in tx.repo().view().local_remote_bookmarks(remote) {
            // We only process tracked bookmarks as we're syncing with remotes
            if local_remote_ref.remote_ref.is_tracked() {
                // For sync operations, we need to capture where the remote bookmark
                // was pointing BEFORE the fetch. This is critical for correctly
                // identifying which commits need to be rebased.
                //
                // Why use remote_ref.target instead of local_target?
                // - remote_ref.target: The actual remote bookmark position (what we need)
                // - local_target: Where the user moved their local bookmark (not relevant for rebase base)
                //
                // Example scenario:
                // 1. Remote has commit A, fetch creates: origin@origin -> A, origin -> A
                // 2. User creates commit B and moves origin -> B
                // 3. Remote gets commit C
                // 4. Before fetch: origin@origin -> A (correct base), origin -> B (user's position)
                // 5. We must rebase descendants of A onto C, not descendants of B
                
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
                            // Fall back to the local target which should be visible.
                            //
                            // Why this fallback is safe:
                            // - If remote had commit A, then was force-pushed removing A
                            // - local_target still points to A (or a descendant)
                            // - Using local_target ensures we have a valid commit for rebasing
                            if let Some(local_id) = local_remote_ref.local_target.as_normal() {
                                let symbol = name.to_remote_symbol(remote).to_owned();
                                pre_fetch_heads.insert(symbol, local_id.clone());
                                tracing::debug!(
                                    ?name,
                                    ?commit_id,
                                    ?local_id,
                                    "Remote bookmark points to hidden commit, using local target as fallback"
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
    let mut rebase_operations: Vec<(String, CommitId, CommitId)> = Vec::new();

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

                // We need to rebase local commits that were descendants of old_head_id
                // but are not ancestors of new_head_id
                rebase_operations.push((
                    symbol.as_ref().to_string(),
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

    for (symbol_str, old_head_id, new_head_id) in rebase_operations {
        writeln!(
            ui.status(),
            "Rebasing local commits from {symbol_str} ({} -> {})",
            old_head_id.hex(),
            new_head_id.hex()
        )?;

        // Find commits that need to be rebased: descendants of old_head that are
        // not ancestors of new_head AND not descendants of other remote bookmarks
        let mut old_head_descendants_revset = RevsetExpression::commit(old_head_id.clone())
            .descendants()
            .minus(&RevsetExpression::commit(new_head_id.clone()).ancestors());

        // Exclude commits that are descendants of other remote bookmark heads
        // This prevents us from accidentally rebasing commits that belong to other
        // bookmarks
        for (other_symbol, other_old_head) in &pre_fetch_heads {
            if other_symbol.as_ref().to_string() != symbol_str && other_old_head != &old_head_id {
                old_head_descendants_revset = old_head_descendants_revset
                    .minus(&RevsetExpression::commit(other_old_head.clone()).descendants());
            }
        }

        let commits_to_rebase = match old_head_descendants_revset.evaluate(tx.repo()) {
            Ok(revset) => revset.iter().collect::<Result<Vec<_>, _>>(),
            Err(err) => return Err(user_error(format!("Revset evaluation failed: {err}"))),
        }?;

        if commits_to_rebase.is_empty() {
            writeln!(ui.status(), "  No local commits to rebase for {symbol_str}")?;
            continue;
        }

        writeln!(
            ui.status(),
            "  Rebasing {} commits",
            commits_to_rebase.len()
        )?;

        // Log which bookmarks are on the commits being rebased
        tracing::debug!(?symbol_str, num_commits = commits_to_rebase.len(), "Rebasing commits");
        for commit_id in &commits_to_rebase {
            let commit = tx.repo().store().get_commit(commit_id)?;
            let bookmarks_at_commit: Vec<String> = tx
                .repo()
                .view()
                .local_bookmarks()
                .filter_map(|(name, target)| {
                    if target.added_ids().any(|id| id == commit_id) {
                        Some(name.as_str().to_string())
                    } else {
                        None
                    }
                })
                .collect();

            if !bookmarks_at_commit.is_empty() {
                tracing::debug!(
                    commit_hex = ?commit_id.hex(),
                    commit_summary = ?commit.description().lines().next().unwrap_or(""),
                    ?bookmarks_at_commit,
                    "Commit has bookmarks"
                );
            }
        }

        // Record the rewrite for these commits to rebase them onto new_head_id
        tracing::debug!(
            num_commits = commits_to_rebase.len(),
            new_head_hex = ?new_head_id.hex(),
            "Setting rewrite mapping"
        );
        for commit_id in &commits_to_rebase {
            tx.repo_mut()
                .set_rewritten_commit(commit_id.clone(), new_head_id.clone());
        }

        // Configure rebase options to drop empty commits
        let rebase_options = RebaseOptions {
            empty: jj_lib::rewrite::EmptyBehaviour::AbandonAllEmpty,
            ..Default::default()
        };

        // Perform the rebase
        let mut commits_rebased_in_stack = 0;
        tx.repo_mut().rebase_descendants_with_options(
            &rebase_options,
            |_old_commit, _rebased_commit| {
                commits_rebased_in_stack += 1;
            },
        )?;

        total_rebased_commits += commits_rebased_in_stack;
        total_abandoned_commits += commits_to_rebase.len() - commits_rebased_in_stack;
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

    // Log final state of all local bookmarks
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
