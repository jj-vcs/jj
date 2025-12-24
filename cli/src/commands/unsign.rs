// Copyright 2023 The Jujutsu Authors
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

use clap_complete::ArgValueCompleter;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetIteratorExt as _;
use jj_lib::signing::SignBehavior;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Drop a cryptographic signature
///
/// See also [commit signing] docs.
///
/// [commit signing]:
///     https://docs.jj-vcs.dev/latest/config/#commit-signing
#[derive(clap::Args, Clone, Debug)]
pub struct UnsignArgs {
    /// What revision(s) to unsign
    #[arg(long, short, value_name = "REVSETS")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    revisions: Vec<RevisionArg>,
}

pub fn cmd_unsign(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UnsignArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let target_expr = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .resolve()?;
    workspace_command.check_rewritable_expr(&target_expr)?;
    let commits: IndexSet<Commit> = target_expr
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .commits(workspace_command.repo().store())
        .try_collect()?;

    let to_unsign: IndexSet<Commit> = commits
        .into_iter()
        .filter(|commit| commit.is_signed())
        .collect();

    let mut tx = workspace_command.start_transaction();

    let mut unsigned_commits = vec![];
    let mut num_reparented = 0;

    tx.repo_mut().transform_descendants(
        to_unsign.iter().ids().cloned().collect_vec(),
        async |rewriter| {
            let old_commit = rewriter.old_commit().clone();
            let commit_builder = rewriter.reparent();

            if to_unsign.contains(&old_commit) {
                let new_commit = commit_builder
                    .set_sign_behavior(SignBehavior::Drop)
                    .write()?;

                unsigned_commits.push(new_commit);
            } else {
                commit_builder.write()?;
                num_reparented += 1;
            }
            Ok(())
        },
    )?;

    if let Some(mut formatter) = ui.status_formatter()
        && !unsigned_commits.is_empty()
    {
        writeln!(formatter, "Unsigned {} commits:", unsigned_commits.len())?;
        print_updated_commits(
            formatter.as_mut(),
            &tx.commit_summary_template(),
            &unsigned_commits,
        )?;
    }

    let num_not_authored_by_me = unsigned_commits
        .iter()
        .filter(|commit| commit.author().email != tx.settings().user_email())
        .count();
    if num_not_authored_by_me > 0 {
        writeln!(
            ui.warning_default(),
            "{num_not_authored_by_me} of these commits are not authored by you",
        )?;
    }

    if num_reparented > 0 {
        writeln!(ui.status(), "Rebased {num_reparented} descendant commits")?;
    }

    let transaction_description = match &*unsigned_commits {
        [] => "".to_string(),
        [commit] => format!("unsign commit {}", commit.id()),
        commits => format!(
            "unsign commit {} and {} more",
            commits[0].id(),
            commits.len() - 1
        ),
    };
    tx.finish(ui, transaction_description)?;

    Ok(())
}
