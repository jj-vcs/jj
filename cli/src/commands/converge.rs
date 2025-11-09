use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::get_divergent_commits;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Resolves divergent changes.
///
/// Attempts to resolve divergence by replacing the visible commits for a given
/// divergent change-id with a single commit.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct ConvergeArgs {}

pub(crate) fn cmd_converge(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &ConvergeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let revs = workspace_command
        .settings()
        .get_string("revsets.converge")?;
    let target_expr = workspace_command
        .parse_revset(ui, &RevisionArg::from(revs))?
        .resolve()?;

    workspace_command.check_rewritable_expr(&target_expr)?;
    let repo = workspace_command.repo();

    let divergent_commits = get_divergent_commits(repo, target_expr)?;
    if let Some(change_id) = choose_change_id(ui, &divergent_commits)? {
        converge_change(ui, change_id, &mut workspace_command)?;
    }
    Ok(())
}

fn converge_change(
    ui: &mut Ui,
    change_id: ChangeId,
    workspace_command: &mut WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    writeln!(
        ui.status(),
        "Resolving divergence on change {}",
        change_id.reverse_hex()
    )?;

    // TODO(drieber): implement converge logic
    // TODO(drieber): Find other visible commits for change_id.
    // TODO(drieber): Build truncated evolution graph for the divergent commits.
    // TODO(drieber): Build MergedState.

    let tx = workspace_command.start_transaction();
    tx.finish(ui, "converge commits")?;

    Ok(())
}

fn choose_change_id(
    ui: &mut Ui,
    divergent_commits: &CommitsByChangeId,
) -> Result<Option<ChangeId>, CommandError> {
    if divergent_commits.is_empty() {
        writeln!(ui.status(), "There are no divergent changes.")?;
        return Ok(None);
    }

    if divergent_commits.len() == 1 {
        let (change_id, commits) = divergent_commits.iter().next().unwrap();
        // TODO(drieber): list the commit ids.
        let commit_ids = commits.iter().map(|c| c.id()).take(10).join(", ");
        writeln!(
            ui.status(),
            "Found one divergent change: {} with {} commits: {}",
            change_id.reverse_hex(),
            commits.len(),
            commit_ids,
        )?;
        return Ok(Some(change_id.clone()));
    }

    writeln!(ui.status(), "Multiple divergent changes found:")?;

    let mut formatter = ui.stderr_formatter();
    let mut choices: Vec<String> = Default::default();
    let change_ids: Vec<&ChangeId> = divergent_commits.keys().collect();
    for (i, change_id) in change_ids.iter().enumerate() {
        writeln!(formatter, "{}: {}", i + 1, change_id.reverse_hex())?;
        choices.push(format!("{}", i + 1));
    }
    writeln!(formatter, "q: quit the prompt")?;
    choices.push("q".to_string());
    drop(formatter);
    let index = ui.prompt_choice("Enter the index of the change to converge", &choices, None)?;
    if index > change_ids.len() {
        writeln!(ui.status(), "No change selected, quitting...")?;
        Ok(None)
    } else {
        Ok(Some(change_ids[index].clone()))
    }
}
