// Copyright 2020 The Jujutsu Authors
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

use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::object_id::ObjectId as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::complete;
use crate::text_util::parse_author;
use crate::ui::Ui;

/// Modify the metadata of a revision without changing its content
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct TouchArgs {
    /// The revision(s) to touch (default: @)
    #[arg(
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable)
    )]
    revisions_pos: Vec<RevisionArg>,

    #[arg(
        short = 'r',
        hide = true,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable)
    )]
    revisions_opt: Vec<RevisionArg>,

    /// Update the author timestamp
    ///
    /// This update the author date to now, without modifying the author.
    #[arg(long)]
    update_author_timestamp: bool,

    /// Update the author to the configured user
    ///
    /// This updates the author name and email. The author timestamp is
    /// not modified – use --update-author-timestamp to update the author
    /// timestamp.
    ///
    /// You can use it in combination with the JJ_USER and JJ_EMAIL
    /// environment variables to set a different author:
    ///
    /// $ JJ_USER='Foo Bar' JJ_EMAIL=foo@bar.com jj touch --update-author
    #[arg(long)]
    update_author: bool,

    /// Set author to the provided string
    ///
    /// This changes author name and email while retaining author
    /// timestamp for non-discardable commits.
    #[arg(
        long,
        conflicts_with = "update_author",
        value_parser = parse_author
    )]
    author: Option<(String, String)>,

    /// Preserve the committer
    #[arg(long)]
    preserve_committer: bool,

    /// Preserve the committer timestamp
    #[arg(long)]
    preserve_committer_timestamp: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_touch(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TouchArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let commits: Vec<_> = if !args.revisions_pos.is_empty() || !args.revisions_opt.is_empty() {
        workspace_command
            .parse_union_revsets(ui, &[&*args.revisions_pos, &*args.revisions_opt].concat())?
    } else {
        workspace_command.parse_revset(ui, &RevisionArg::AT)?
    }
    .evaluate_to_commits()?
    .try_collect()?; // in reverse topological order
    let commits: HashMap<_, _> = commits.into_iter().map(|c| (c.id().clone(), c)).collect();

    if commits.is_empty() {
        writeln!(ui.status(), "No revisions to reset.")?;
        return Ok(());
    }
    let commit_ids: Vec<_> = commits.keys().cloned().collect();
    workspace_command.check_rewritable(commit_ids.iter())?;

    let mut tx = workspace_command.start_transaction();
    let tx_description = match &commit_ids[..] {
        [] => unreachable!(),
        [commit] => format!("reset commit {}", commit.hex()),
        [first_commit, remaining_commits @ ..] => {
            format!(
                "reset commit {} and {} more",
                first_commit.hex(),
                remaining_commits.len()
            )
        }
    };

    let mut num_touched = 0;
    let mut num_reparented = 0;
    let mut touched: HashMap<CommitId, CommitId> = HashMap::new();
    tx.repo_mut()
        .transform_descendants(commit_ids, async |rewriter| {
            let old_commit_id = rewriter.old_commit().id().clone();
            let mut commit_builder = rewriter.reparent();
            let new_parents = commit_builder
                .parents()
                .iter()
                .map(|p| (touched.get(p).unwrap_or(p)))
                .cloned()
                .collect();
            commit_builder = commit_builder.set_parents(new_parents);
            if let Some(old_commit) = commits.get(&old_commit_id) {
                let mut new_author = commit_builder.author().clone();
                if let Some((name, email)) = args.author.clone() {
                    new_author.name = name;
                    new_author.email = email;
                } else if args.update_author {
                    new_author.name = commit_builder.committer().name.clone();
                    new_author.email = commit_builder.committer().email.clone();
                }
                if args.update_author_timestamp {
                    new_author.timestamp = commit_builder.committer().timestamp;
                }
                commit_builder = commit_builder.set_author(new_author);

                let mut new_committer = commit_builder.committer().clone();
                if args.preserve_committer {
                    new_committer.name = old_commit.committer().name.clone();
                    new_committer.email = old_commit.committer().email.clone();
                }
                if args.preserve_committer_timestamp {
                    new_committer.timestamp = old_commit.committer().timestamp;
                }
                commit_builder = commit_builder.set_committer(new_committer);

                let new_commit = commit_builder.write()?;
                touched.insert(old_commit_id.clone(), new_commit.id().clone());
                num_touched += 1;
            } else {
                commit_builder.write()?;
                num_reparented += 1;
            }
            Ok(())
        })?;
    for (old, new) in touched {
        tx.repo_mut().set_rewritten_commit(old.clone(), new.clone());
    }
    if num_touched > 1 {
        writeln!(ui.status(), "Updated {num_touched} commits")?;
    }
    if num_reparented > 0 {
        writeln!(ui.status(), "Rebased {num_reparented} descendant commits")?;
    }
    tx.finish(ui, tx_description)?;
    Ok(())
}
