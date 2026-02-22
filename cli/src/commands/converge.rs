use std::collections::HashSet;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::ConvergeError;
use jj_lib::converge::ConvergeResult;
use jj_lib::converge::ConvergeUI;
use jj_lib::converge::converge;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Resolves divergent changes.
///
/// Attempts to resolve divergence by replacing the visible commits for a given
/// divergent change-id with a single commit.
///
/// See <https://github.com/jj-vcs/jj/blob/main/docs/design/jj-converge-command.md> for more details.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct ConvergeArgs {
    /// Limit number of evolog nodes to traverse.
    #[arg(long, default_value_t = 50)]
    max_evolution_nodes: usize,
}

pub(crate) fn cmd_converge(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConvergeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let converge_revset = workspace_command
        .settings()
        .get_string("revsets.converge")?;
    let target_expr = workspace_command
        .parse_revset(ui, &RevisionArg::from(converge_revset))?
        .resolve()?;

    workspace_command.check_rewritable_expr(&target_expr)?;
    let repo = workspace_command.repo();

    let mut converge_ui = ConvergeCommandUI { ui };
    // let tx = workspace_command.start_transaction();

    let solution = match converge(
        &repo,
        Some(&mut converge_ui),
        target_expr,
        args.max_evolution_nodes,
    ) {
        Ok(ConvergeResult::ProposedSolution(solution)) => solution,
        Err(ConvergeError::NoDivergentChanges()) => {
            writeln!(ui.status(), "No divergent changes found.")?;
            return Ok(());
        }
        Err(ConvergeError::UserAborted()) => {
            writeln!(ui.status(), "User aborted converge.")?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    writeln!(
        ui.status(),
        "Resolving divergence on change {}",
        solution.change_id.reverse_hex()
    )?;

    // tx.finish(ui, "converge commits")?;

    Ok(())
}

struct ConvergeCommandUI<'a> {
    ui: &'a mut Ui,
}

impl ConvergeUI for ConvergeCommandUI<'_> {
    fn choose_change_id(
        &self,
        divergent_commits: &CommitsByChangeId,
    ) -> Result<Option<ChangeId>, ConvergeError> {
        if divergent_commits.is_empty() {
            return Ok(None);
        }

        if divergent_commits.len() == 1 {
            let (change_id, commits) = divergent_commits.iter().next().unwrap();
            // TODO(drieber): list the commit ids and change-id in a user friendly way.
            let commit_ids = commits.iter().map(|c| c.id()).take(10).join(", ");
            writeln!(
                self.ui.status(),
                "Found one divergent change: {} with {} commits: {}",
                change_id.reverse_hex(),
                commits.len(),
                commit_ids,
            )?;
            return Ok(Some(change_id.clone()));
        }

        writeln!(self.ui.status(), "Multiple divergent changes found:")?;

        let mut formatter = self.ui.stderr_formatter();
        let mut choices: Vec<String> = Default::default();
        let change_ids: Vec<&ChangeId> = divergent_commits.keys().collect();
        for (i, change_id) in change_ids.iter().enumerate() {
            writeln!(formatter, "{}: {}", i + 1, change_id.reverse_hex())?;
            choices.push(format!("{}", i + 1));
        }
        writeln!(formatter, "q: quit the prompt")?;
        choices.push("q".to_string());
        drop(formatter);
        let index =
            self.ui
                .prompt_choice("Enter the index of the change to converge", &choices, None)?;
        if index > change_ids.len() {
            writeln!(self.ui.status(), "No change selected, quitting...")?;
            Ok(None)
        } else {
            Ok(Some(change_ids[index].clone()))
        }
    }

    fn choose_author(
        &self,
        _divergent_commits: &HashSet<Commit>,
        _evolution_fork_point: Option<&Commit>,
    ) -> Result<CommitId, ConvergeError> {
        todo!()
    }

    fn merge_description_text(
        &self,
        _divergent_commits: &HashSet<Commit>,
        _evolution_fork_point: Option<&Commit>,
    ) -> Result<String, ConvergeError> {
        todo!()
    }

    fn choose_parents(
        &self,
        divergent_commits: &HashSet<Commit>,
    ) -> Result<CommitId, ConvergeError> {
        todo!()
    }
}
