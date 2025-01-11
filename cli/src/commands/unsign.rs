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

/// Drop a cryptographic signature
#[derive(clap::Args, Clone, Debug)]
pub struct UnsignArgs {
    /// What revision(s) to unsign
    #[arg(
        long, short,
        value_name = "REVSETS",
        add = ArgValueCandidates::new(complete::mutable_revisions),
    )]
    revisions: Vec<RevisionArg>,
}

pub fn cmd_unsign(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UnsignArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let commits: IndexSet<Commit> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;

    workspace_command.check_rewritable(commits.iter().ids())?;

    let mut tx = workspace_command.start_transaction();

    let mut unsigned_commits = vec![];
    tx.repo_mut().transform_descendants(
        commits.iter().ids().cloned().collect_vec(),
        |rewriter| {
            if commits.contains(rewriter.old_commit()) {
                let commit_builder = rewriter.reparent();
                let new_commit = commit_builder
                    .set_sign_behavior(SignBehavior::Drop)
                    .write()?;
                unsigned_commits.push(new_commit);
            }
            Ok(())
        },
    )?;

    if let Some(mut formatter) = ui.status_formatter() {
        match &*unsigned_commits {
            [] => {}
            [commit] => {
                write!(formatter, "Unsigned commit ")?;
                tx.base_workspace_helper()
                    .write_commit_summary(formatter.as_mut(), commit)?;
                writeln!(ui.status())?;
            }
            commits => {
                let template = tx.base_workspace_helper().commit_summary_template();
                writeln!(formatter, "Unsigned the following commits:")?;
                for commit in commits {
                    write!(formatter, "  ")?;
                    template.format(commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }
            }
        };
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
