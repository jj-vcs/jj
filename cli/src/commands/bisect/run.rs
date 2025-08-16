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

use clap_complete::ArgValueCompleter;
use jj_lib::bisect::BisectionResult;
use jj_lib::bisect::Bisector;
use jj_lib::bisect::TestResult;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
use crate::config::CommandNameAndArgs;
use crate::ui::Ui;

/// Automatically bisect by testing revisions using a given command.
///
/// It is assumed that if the bug is present at a given revision, then it's also
/// present at all descendant revisions in the input range.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct BisectRunArgs {
    /// Range of revisions to bisect
    ///
    /// This is typically a range like `v1.0..main`. The heads of the range are
    /// assumed to be bad.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    range: Vec<RevisionArg>,
    /// Command to run to determine whether the bug is present
    ///
    /// The command will be run from the workspace root. The exit status of the
    /// command will be used to mark revisions as good or bad:
    /// status 0 means good, 125 means to skip the revision, 127 (command not
    /// found) will abort the bisection, and any other non-zero exit status
    /// means the revision is bad.
    ///
    /// The test target's commit ID is available to the command in the
    /// `$JJ_BISECT_TARGET` environment variable.
    #[arg(long, value_name = "COMMAND", required = true)]
    command: CommandNameAndArgs,
}

#[instrument(skip_all)]
pub(crate) fn cmd_bisect_run(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BisectRunArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let input_range = workspace_command
        .parse_union_revsets(ui, &args.range)?
        .resolve()?;

    let initial_repo = workspace_command.repo().clone();

    let mut bisector = Bisector::new(initial_repo.as_ref(), input_range)?;
    let bisection_result = loop {
        match bisector.next_step()? {
            jj_lib::bisect::NextStep::Test(commit) => {
                if let Some(mut formatter) = ui.status_formatter() {
                    // TODO: Show a graph of the current range instead?
                    // TODO: Say how many commits are left and estimate the number of iterations.
                    let commit_template = workspace_command.commit_summary_template();
                    write!(formatter, "Now testing: ")?;
                    commit_template.format(&commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }

                let test_result = test_commit(ui, &mut workspace_command, &args.command, &commit)?;

                if let Some(mut formatter) = ui.status_formatter() {
                    let message = match test_result {
                        TestResult::Good => "The commit is good.",
                        TestResult::Bad => "The commit is bad.",
                        TestResult::Skip => {
                            "It could not be determine if the commit is good or bad."
                        }
                    };
                    writeln!(formatter, "{message}")?;
                    writeln!(formatter)?;
                }

                bisector.mark(commit.id().clone(), test_result);

                // Reload the workspace because the test command may run `jj` commands.
                workspace_command = command.workspace_helper(ui)?;
            }
            jj_lib::bisect::NextStep::Done(bisection_result) => {
                break bisection_result;
            }
        }
    };

    match bisection_result {
        BisectionResult::Indeterminate => {
            return Err(user_error(
                "Could not find the first bad commit. Was the input range empty?",
            ));
        }
        BisectionResult::Found(first_bad_commits) => {
            let commit_template = workspace_command.commit_summary_template();
            let mut formatter = ui.stdout_formatter();
            if let [first_bad_commit] = first_bad_commits.as_slice() {
                write!(formatter, "The first bad commit is: ")?;
                commit_template.format(first_bad_commit, formatter.as_mut())?;
                writeln!(formatter)?;
            } else {
                writeln!(formatter, "The first bad commits are:")?;
                for first_bad_commit in first_bad_commits {
                    commit_template.format(&first_bad_commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }
            }
        }
    }

    Ok(())
}

fn test_commit(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    command: &CommandNameAndArgs,
    commit: &Commit,
) -> Result<TestResult, CommandError> {
    let mut tx = workspace_command.start_transaction();
    let commit_id_hex = commit.id().hex();
    tx.check_out(commit)?;
    tx.finish(
        ui,
        format!("Checked out commit {commit_id_hex} for bisection"),
    )?;

    let mut cmd = command.to_command();
    tracing::info!(?cmd, "running bisection test command");
    let status = cmd
        .env("JJ_BISECT_TARGET", &commit_id_hex)
        .status()
        .map_err(|err| user_error_with_message("Failed to run test command", err))?;
    let test_result = if status.success() {
        TestResult::Good
    } else {
        match status.code() {
            Some(125) => TestResult::Skip,
            Some(127) => {
                return Err(user_error(
                    "Test command returned 127 (command not found) - aborting bisection.",
                ));
            }
            _ => TestResult::Bad,
        }
    };

    // TODO: Should we abandon the working copy here? If the test script wrote files
    // to the working copy, the user probably doesn't want to keep those around.
    // Another option to restore the repo at the end of the whole bisection process
    // to the pre-bisection state.

    Ok(test_result)
}
