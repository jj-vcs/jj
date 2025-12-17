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
use jj_lib::bisect::Evaluation;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::short_operation_hash;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::internal_error_with_message;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
use crate::config::CommandNameAndArgs;
use crate::ui::Ui;

/// Run a given command to find the first bad revision.
///
/// Uses binary search to find the first bad revision. Revisions are evaluated
/// by running a given command (see the documentation for `--command` for
/// details).
///
/// It is assumed that if a given revision is bad, then all its descendants
/// in the input range are also bad.
///
/// The target of the bisection can be inverted to look for the first good
/// revision by passing `--find-good`.
///
/// Hint: You can pass your shell as evaluation command. You can then run
/// manual tests in the shell and make sure to exit the shell with appropriate
/// error code depending on the outcome (e.g. `exit 0` to mark the revision as
/// good in Bash or Fish).
///
/// Example: To run `cargo test` with the changes from revision `xyz` applied:
///
/// `jj bisect --range v1.0..main -- bash -c "jj duplicate -r xyz -B @ && cargo
/// test"`
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct BisectRunArgs {
    /// Range of revisions to bisect
    ///
    /// This is typically a range like `v1.0..main`. The heads of the range are
    /// assumed to be bad. Ancestors of the range that are not also in the range
    /// are assumed to be good.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        required = true,
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    range: Vec<RevisionArg>,
    /// Deprecated. Use positional arguments instead.
    #[arg(
        long = "command",
        value_name = "COMMAND",
        hide = true,
        conflicts_with = "command"
    )]
    legacy_command: Option<CommandNameAndArgs>,

    /// Command to run to determine whether the bug is present
    ///
    /// The exit status of the command will be used to mark revisions as good or
    /// bad: status 0 means good, 125 means to skip the revision, 127 (command
    /// not found) will abort the bisection, and any other non-zero exit status
    /// means the revision is bad.
    ///
    /// The target's commit ID is available to the command in the
    /// `$JJ_BISECT_TARGET` environment variable.
    #[arg(value_name = "COMMAND")]
    command: Option<String>,

    /// Arguments to pass to the command
    ///
    /// Hint: Use a `--` separator to allow passing arguments starting with `-`.
    /// For example `jj bisect run --range=... -- test -f some-file`.
    #[arg(value_name = "ARGS")]
    args: Vec<String>,

    /// Whether to find the first good revision instead
    ///
    /// Inverts the interpretation of exit statuses (excluding special exit
    /// statuses).
    #[arg(long, value_name = "TARGET", default_value = "false")]
    find_good: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_bisect_run(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BisectRunArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    if let Some(command) = &args.legacy_command {
        writeln!(
            ui.warning_default(),
            "`--command` is deprecated; use positional arguments instead: `jj bisect run \
             --range=... -- {command}`"
        )?;
    } else if args.command.is_none() {
        return Err(cli_error("Command argument is required"));
    }

    let input_range = workspace_command
        .parse_union_revsets(ui, &args.range)?
        .resolve()?;

    let initial_repo = workspace_command.repo().clone();

    let mut bisector = Bisector::new(initial_repo.as_ref(), input_range)?;
    let bisection_result = loop {
        match bisector.next_step()? {
            jj_lib::bisect::NextStep::Evaluate(commit) => {
                {
                    let mut formatter = ui.stdout_formatter();
                    // TODO: Show a graph of the current range instead?
                    // TODO: Say how many commits are left and estimate the number of iterations.
                    let commit_template = workspace_command.commit_summary_template();
                    write!(formatter, "Now evaluating: ")?;
                    commit_template.format(&commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }

                let cmd = get_command(args);
                let evaluation = evaluate_commit(ui, &mut workspace_command, cmd, &commit)?;

                {
                    let mut formatter = ui.stdout_formatter();
                    let message = match evaluation {
                        Evaluation::Good => "The revision is good.",
                        Evaluation::Bad => "The revision is bad.",
                        Evaluation::Skip => {
                            "It could not be determined if the revision is good or bad."
                        }
                    };
                    writeln!(formatter, "{message}")?;
                    writeln!(formatter)?;
                }

                if args.find_good {
                    // If we're looking for the first good revision,
                    // invert the evaluation result.
                    bisector.mark(commit.id().clone(), evaluation.invert());
                } else {
                    bisector.mark(commit.id().clone(), evaluation);
                }

                // Reload the workspace because the evaluation command may run `jj` commands.
                workspace_command = command.workspace_helper(ui)?;
            }
            jj_lib::bisect::NextStep::Done(bisection_result) => {
                break bisection_result;
            }
        }
    };

    let mut formatter = ui.stdout_formatter();
    writeln!(
        formatter,
        "Search complete. To discard any revisions created during search, run:"
    )?;
    writeln!(
        formatter,
        "  jj op restore {}",
        short_operation_hash(initial_repo.op_id())
    )?;

    let target = if args.find_good { "good" } else { "bad" };
    match bisection_result {
        BisectionResult::Indeterminate => {
            return Err(user_error(format!(
                "Could not find the first {target} revision. Was the input range empty?"
            )));
        }
        BisectionResult::Found(first_target_commits) => {
            let commit_template = workspace_command.commit_summary_template();
            if let [first_target_commit] = first_target_commits.as_slice() {
                write!(formatter, "The first {target} revision is: ")?;
                commit_template.format(first_target_commit, formatter.as_mut())?;
                writeln!(formatter)?;
            } else {
                writeln!(formatter, "The first {target} revisions are:")?;
                for first_target_commit in first_target_commits {
                    commit_template.format(&first_target_commit, formatter.as_mut())?;
                    writeln!(formatter)?;
                }
            }
        }
    }

    Ok(())
}

fn get_command(args: &BisectRunArgs) -> std::process::Command {
    if let Some(command) = &args.command {
        let mut cmd = std::process::Command::new(command);
        cmd.args(&args.args);
        cmd
    } else {
        args.legacy_command.as_ref().unwrap().to_command()
    }
}

fn evaluate_commit(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    mut cmd: std::process::Command,
    commit: &Commit,
) -> Result<Evaluation, CommandError> {
    let mut tx = workspace_command.start_transaction();
    let commit_id_hex = commit.id().hex();
    tx.check_out(commit)?;
    tx.finish(
        ui,
        format!("Updated to revision {commit_id_hex} for bisection"),
    )?;

    let jj_executable_path = std::env::current_exe().map_err(|err| {
        internal_error_with_message("Could not get path for the jj executable", err)
    })?;
    tracing::info!(?cmd, "running bisection evaluation command");
    let status = cmd
        .env("JJ_EXECUTABLE_PATH", jj_executable_path)
        .env("JJ_BISECT_TARGET", &commit_id_hex)
        .status()
        .map_err(|err| user_error_with_message("Failed to run evaluation command", err))?;
    let evaluation = if status.success() {
        Evaluation::Good
    } else {
        match status.code() {
            Some(125) => Evaluation::Skip,
            Some(127) => {
                return Err(user_error(
                    "Evaluation command returned 127 (command not found) - aborting bisection.",
                ));
            }
            _ => Evaluation::Bad,
        }
    };

    Ok(evaluation)
}
