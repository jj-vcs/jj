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
use indexmap::IndexSet;
use itertools::Itertools;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt;
use jj_lib::signing::SignBehavior;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
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
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::mutable_revisions),
    )]
    revisions: Vec<RevisionArg>,
}

pub fn cmd_sign(ui: &mut Ui, command: &CommandHelper, args: &SignArgs) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let commits: IndexSet<Commit> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;

    workspace_command.check_rewritable(commits.iter().ids())?;

    let mut tx = workspace_command.start_transaction();

    let is_signed_by_me = |commit: &Commit| -> bool {
        commit.is_signed() && commit.author().email == command.settings().user_email()
    };

    let to_sign = commits
        .iter()
        .filter(|commit| !is_signed_by_me(commit))
        .ids()
        .cloned()
        .collect_vec();

    let mut signed_commits = vec![];
    let mut foreign_commits = vec![];
    tx.repo_mut().transform_descendants(to_sign, |rewriter| {
        if is_signed_by_me(rewriter.old_commit()) {
            // Don't resign commits, which are already signed by me. For people using
            // hardware devices for signatures, resigning commits is
            // cumbersome.
            return Ok(());
        }

        let authored_by_me =
            rewriter.old_commit().author().email == command.settings().user_email();

        let old_commit = rewriter.old_commit().clone();
        let commit_builder = rewriter.reparent();
        if commits.contains(&old_commit) {
            let new_commit = commit_builder
                .set_sign_key(args.key.clone())
                .set_sign_behavior(SignBehavior::Own)
                .write()?;
            signed_commits.push(new_commit.clone());

            if !authored_by_me {
                foreign_commits.push(new_commit);
            }
        } else {
            commit_builder.write()?;
        }

        Ok(())
    })?;

    if let Some(mut formatter) = ui.status_formatter() {
        match &*foreign_commits {
            [] => {}
            [commit] => {
                write!(ui.warning_default(), "Signed 1 commit not authored by you")?;
                tx.base_workspace_helper()
                    .write_commit_summary(formatter.as_mut(), commit)?;
                writeln!(ui.status())?;
            }
            commits => {
                let template = tx.base_workspace_helper().commit_summary_template();
                writeln!(
                    ui.warning_default(),
                    "Signed {} commits not authored by you",
                    commits.len()
                )?;
                for commit in commits {
                    write!(formatter, "  ")?;
                    template.format(commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }
            }
        };

        match &*signed_commits {
            [] => {}
            [commit] => {
                write!(formatter, "Signed 1 commit ")?;
                tx.base_workspace_helper()
                    .write_commit_summary(formatter.as_mut(), commit)?;
                writeln!(ui.status())?;
            }
            commits => {
                let template = tx.base_workspace_helper().commit_summary_template();
                writeln!(formatter, "Signed {} commits:", commits.len())?;
                for commit in commits {
                    write!(formatter, "  ")?;
                    template.format(commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }
            }
        };
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
