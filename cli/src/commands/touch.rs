// Copyright 2025 The Jujutsu Authors
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

use std::collections::HashSet;

use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
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
}

#[instrument(skip_all)]
pub(crate) fn cmd_touch(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TouchArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let commit_ids: Vec<_> = if !args.revisions_pos.is_empty() || !args.revisions_opt.is_empty() {
        workspace_command
            .parse_union_revsets(ui, &[&*args.revisions_pos, &*args.revisions_opt].concat())?
    } else {
        workspace_command.parse_revset(ui, &RevisionArg::AT)?
    }
    .evaluate_to_commit_ids()?
    .try_collect()?;
    if commit_ids.is_empty() {
        writeln!(ui.status(), "No revisions to touch.")?;
        return Ok(());
    }
    workspace_command.check_rewritable(commit_ids.iter())?;

    let mut tx = workspace_command.start_transaction();
    let tx_description = match commit_ids.as_slice() {
        [] => unreachable!(),
        [commit] => format!("touch commit {}", commit.hex()),
        [first_commit, remaining_commits @ ..] => {
            format!(
                "touch commit {} and {} more",
                first_commit.hex(),
                remaining_commits.len()
            )
        }
    };

    let mut num_touched = 0;
    let mut num_reparented = 0;
    let commit_ids_set: HashSet<_> = commit_ids.iter().cloned().collect();
    // Even though `MutableRepo::rewrite_commit` and
    // `MutableRepo::rebase_descendants` can handle rewriting of a commit even
    // if it is a descendant of another commit being rewritten, using
    // `MutableRepo::transform_descendants` prevents us from rewriting the same
    // commit multiple times, and adding additional entries in the predecessor
    // chain.
    tx.repo_mut()
        .transform_descendants(commit_ids, async |rewriter| {
            let old_commit_id = rewriter.old_commit().id().clone();
            let mut commit_builder = rewriter.reparent();
            if commit_ids_set.contains(&old_commit_id) {
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

                commit_builder.write()?;
                num_touched += 1;
            } else {
                commit_builder.write()?;
                num_reparented += 1;
            }
            Ok(())
        })?;
    if num_touched > 0 {
        writeln!(ui.status(), "Updated {num_touched} commits")?;
    }
    if num_reparented > 0 {
        writeln!(ui.status(), "Rebased {num_reparented} descendant commits")?;
    }
    tx.finish(ui, tx_description)?;
    Ok(())
}
