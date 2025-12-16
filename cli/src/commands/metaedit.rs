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
use jj_lib::backend::Timestamp;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use jj_lib::time_util::parse_datetime;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::complete;
use crate::description_util::join_message_paragraphs;
use crate::text_util::parse_author;
use crate::ui::Ui;

/// Modify the metadata of a revision without changing its content
///
/// Whenever any metadata is updated, the committer name, email, and timestamp
/// are also updated for all rebased commits. The name and email may come from
/// the `JJ_USER` and `JJ_EMAIL` environment variables, as well as by passing
/// `--config user.name` and `--config user.email`.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct MetaeditArgs {
    /// The revision(s) to modify (default: @) [aliases: -r]
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

    /// Generate a new change-id
    ///
    /// This generates a new change-id for the revision.
    #[arg(long)]
    update_change_id: bool,

    /// Update the change description
    ///
    /// This updates the change description, without opening the editor.
    ///
    /// Use `jj describe` if you want to use an editor.
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,

    /// Update the author timestamp
    ///
    /// This updates the author date to the current time, without modifying the
    /// author.
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
    /// $ JJ_USER='Foo Bar' JJ_EMAIL=foo@bar.com jj metaedit --update-author
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

    /// Set the author date to the given date either human
    /// readable, eg Sun, 23 Jan 2000 01:23:45 JST) or as a time stamp, eg
    /// 2000-01-23T01:23:45+09:00)
    #[arg(
        long,
        conflicts_with = "update_author_timestamp",
        value_parser = parse_datetime
    )]
    author_timestamp: Option<Timestamp>,

    /// Rewrite the commit, even if no other metadata changed
    ///
    /// This updates the committer timestamp to the current time, as well as the
    /// committer name and email.
    ///
    /// Even if this option is not passed, the committer name, email, and
    /// timestamp will be updated if other metadata is updated. This option
    /// just forces every commit to be rewritten whether or not there are other
    /// changes.
    ///
    /// You can use it in combination with the `JJ_USER` and `JJ_EMAIL`
    /// environment variables to set a different committer:
    ///
    /// $ JJ_USER='Foo Bar' JJ_EMAIL=foo@bar.com jj metaedit --force-rewrite
    #[arg(long)]
    force_rewrite: bool,

    // TODO: remove in jj 0.41.0+
    /// Deprecated. Use `--force-rewrite` instead.
    #[arg(
        long = "update-committer-timestamp",
        hide = true,
        conflicts_with = "force_rewrite"
    )]
    legacy_update_committer_timestamp: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_metaedit(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &MetaeditArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    if args.legacy_update_committer_timestamp {
        writeln!(
            ui.warning_default(),
            "`--update-committer-timestamp` is deprecated; use `--force-rewrite` instead"
        )?;
    }

    let target_expr = if !args.revisions_pos.is_empty() || !args.revisions_opt.is_empty() {
        workspace_command
            .parse_union_revsets(ui, &[&*args.revisions_pos, &*args.revisions_opt].concat())?
    } else {
        workspace_command.parse_revset(ui, &RevisionArg::AT)?
    }
    .resolve()?;
    workspace_command.check_rewritable_expr(&target_expr)?;
    let commit_ids: Vec<_> = target_expr
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .try_collect()?;
    if commit_ids.is_empty() {
        writeln!(ui.status(), "No revisions to modify.")?;
        return Ok(());
    }

    let mut tx = workspace_command.start_transaction();
    let tx_description = match commit_ids.as_slice() {
        [] => unreachable!(),
        [commit] => format!("edit commit metadata for commit {}", commit.hex()),
        [first_commit, remaining_commits @ ..] => {
            format!(
                "edit commit metadata for commit {} and {} more",
                first_commit.hex(),
                remaining_commits.len()
            )
        }
    };

    let new_description = if !args.message_paragraphs.is_empty() {
        Some(join_message_paragraphs(&args.message_paragraphs))
    } else {
        None
    };

    let mut num_reparented = 0;
    let commit_ids_set: HashSet<_> = commit_ids.iter().cloned().collect();
    let mut modified: Vec<Commit> = Vec::new();
    // Even though `MutableRepo::rewrite_commit` and
    // `MutableRepo::rebase_descendants` can handle rewriting of a commit even
    // if it is a descendant of another commit being rewritten, using
    // `MutableRepo::transform_descendants` prevents us from rewriting the same
    // commit multiple times, and adding additional entries in the predecessor
    // chain.
    tx.repo_mut()
        .transform_descendants(commit_ids, async |rewriter| {
            if commit_ids_set.contains(rewriter.old_commit().id()) {
                let mut rewrite = args.force_rewrite
                    || args.legacy_update_committer_timestamp
                    || rewriter.parents_changed();

                let old_author = rewriter.old_commit().author().clone();
                let mut commit_builder = rewriter.reparent();
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
                if let Some(author_date) = args.author_timestamp {
                    new_author.timestamp = author_date;
                }
                // If the old commit had an unset author, the commit builder
                // may already have the author updated from the current config.
                // Thus, compare to the actual old_author to correctly detect
                // changes.
                if new_author.name != old_author.name
                    || new_author.email != old_author.email
                    || (new_author.timestamp != commit_builder.author().timestamp
                        && new_author.timestamp != old_author.timestamp)
                {
                    commit_builder = commit_builder.set_author(new_author);
                    rewrite = true;
                }

                if let Some(description) = &new_description
                    && description != commit_builder.description()
                {
                    commit_builder = commit_builder.set_description(description);
                    rewrite = true;
                }

                if args.update_change_id {
                    commit_builder = commit_builder.generate_new_change_id();
                    rewrite = true;
                }

                if rewrite {
                    let new_commit = commit_builder.write()?;
                    modified.push(new_commit);
                }
            } else if rewriter.parents_changed() {
                rewriter.reparent().write()?;
                num_reparented += 1;
            }
            Ok(())
        })?;
    if !modified.is_empty() {
        writeln!(ui.status(), "Modified {} commits:", modified.len())?;
        if let Some(mut formatter) = ui.status_formatter() {
            print_updated_commits(formatter.as_mut(), &tx.commit_summary_template(), &modified)?;
        }
    }
    if num_reparented > 0 {
        writeln!(ui.status(), "Rebased {num_reparented} descendant commits")?;
    }
    tx.finish(ui, tx_description)?;
    Ok(())
}
