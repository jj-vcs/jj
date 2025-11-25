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
use crate::command_error::user_error_with_hint;
use crate::complete;
use crate::ui::Ui;

/// Cryptographically sign a revision
///
/// This command requires configuring a [commit signing] backend.
///
/// [commit signing]:
///     https://docs.jj-vcs.dev/latest/config/#commit-signing
#[derive(clap::Args, Clone, Debug)]
pub struct SignArgs {
    /// What revision(s) to sign
    ///
    /// If no revisions are specified, this defaults to the `revsets.sign`
    /// setting.
    ///
    /// Note that revisions are always re-signed.
    ///
    /// While that leads to discomfort for users, which sign with hardware
    /// devices, as of now we cannot reliably check if a commit is already
    /// signed by the user without creating a signature (see [#5786]).
    ///
    /// [#5786]:
    ///     https://github.com/jj-vcs/jj/issues/5786
    #[arg(
        long, short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    revisions: Vec<RevisionArg>,

    /// The key used for signing
    #[arg(long)]
    key: Option<String>,
}

pub fn cmd_sign(ui: &mut Ui, command: &CommandHelper, args: &SignArgs) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    if !workspace_command.repo().store().signer().can_sign() {
        return Err(user_error_with_hint(
            "No signing backend configured",
            "For configuring a signing backend, see https://docs.jj-vcs.dev/latest/config/#commit-signing",
        ));
    }

    let revset_expression = if args.revisions.is_empty() {
        let revset_string = workspace_command.settings().get_string("revsets.sign")?;
        workspace_command.parse_revset(ui, &RevisionArg::from(revset_string))?
    } else {
        workspace_command.parse_union_revsets(ui, &args.revisions)?
    }
    .resolve()?;

    workspace_command.check_rewritable_expr(&revset_expression)?;

    let to_sign: IndexSet<Commit> = revset_expression
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .commits(workspace_command.repo().store())
        .try_collect()?;

    let mut tx = workspace_command.start_transaction();

    let mut signed_commits = vec![];
    let mut num_reparented = 0;

    tx.repo_mut().transform_descendants(
        to_sign.iter().ids().cloned().collect_vec(),
        async |rewriter| {
            let old_commit = rewriter.old_commit().clone();
            let mut commit_builder = rewriter.reparent();

            if to_sign.contains(&old_commit) {
                if let Some(key) = &args.key {
                    commit_builder = commit_builder.set_sign_key(key.clone());
                }

                let new_commit = commit_builder
                    .set_sign_behavior(SignBehavior::Force)
                    .write()?;

                signed_commits.push(new_commit);
            } else {
                commit_builder.write()?;
                num_reparented += 1;
            }

            Ok(())
        },
    )?;

    if let Some(mut formatter) = ui.status_formatter()
        && !signed_commits.is_empty()
    {
        writeln!(formatter, "Signed {} commits:", signed_commits.len())?;
        print_updated_commits(
            formatter.as_mut(),
            &tx.commit_summary_template(),
            &signed_commits,
        )?;
    }

    let num_not_authored_by_me = signed_commits
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

    let transaction_description = match &*signed_commits {
        [] => "".to_string(),
        [commit] => format!("sign commit {}", commit.id()),
        commits => format!(
            "sign commit {} and {} more",
            commits[0].id(),
            commits.len() - 1
        ),
    };
    tx.finish(ui, transaction_description)?;

    Ok(())
}
