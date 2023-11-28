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

use clap_complete::ArgValueCandidates;
use itertools::Itertools;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt;
use jj_lib::signing::SignBehavior;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Cryptographically sign a revision
#[derive(clap::Args, Clone, Debug)]
pub struct SignArgs {
    /// What key to use, depends on the configured signing backend.
    #[arg()]
    key: Option<String>,
    /// What revision(s) to sign
    #[arg(
        long, short,
        default_value = "@",
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::mutable_revisions),
    )]
    revisions: Vec<RevisionArg>,
    /// Sign a commit that is not authored by you or was already signed.
    #[arg(long, short)]
    force: bool,
    /// Drop the signature, explicitly "un-signing" the commit.
    #[arg(long, short = 'D', conflicts_with = "force")]
    drop: bool,
}

pub fn cmd_sign(ui: &mut Ui, command: &CommandHelper, args: &SignArgs) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let commits: Vec<Commit> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;

    let commit_ids = commits.iter().ids().collect_vec();
    workspace_command.check_rewritable(commit_ids)?;

    for commit in &commits {
        if !args.force {
            if !args.drop && commit.is_signed() {
                return Err(user_error(
                    "Commit is already signed, use --force to sign anyway",
                ));
            }
            if commit.author().email != command.settings().user_email() {
                return Err(user_error(
                    "Commit is not authored by you, use --force to sign anyway",
                ));
            }
        }
    }

    let mut tx = workspace_command.start_transaction();

    let behavior = if args.drop {
        SignBehavior::Drop
    } else if args.force {
        SignBehavior::Force
    } else {
        SignBehavior::Own
    };
    let mut signed_commits = vec![];
    tx.repo_mut().transform_descendants(
        commits.iter().ids().cloned().collect_vec(),
        |rewriter| {
            if commits.contains(rewriter.old_commit()) {
                let commit_builder = rewriter.rebase()?;
                let new_commit = commit_builder
                    .set_sign_key(args.key.clone())
                    .set_sign_behavior(behavior)
                    .write()?;
                signed_commits.push(new_commit);
            }
            Ok(())
        },
    )?;

    tx.finish(ui, format!("signed {} commits", signed_commits.len()))?;

    let Some(mut formatter) = ui.status_formatter() else {
        return Ok(());
    };
    let template = workspace_command.commit_summary_template();
    for commit in &signed_commits {
        if args.drop {
            write!(formatter, "Signature was dropped: ")?;
        } else {
            write!(formatter, "Commit was signed: ")?;
        }
        template.format(commit, formatter.as_mut())?;
        writeln!(formatter)?;
    }

    Ok(())
}
