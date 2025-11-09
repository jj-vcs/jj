// Copyright 2026 The Jujutsu Authors
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

use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::conflicts::materialize_merge_result_to_bytes;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::ConvergeError;
use jj_lib::converge::ConvergeUI;
use jj_lib::converge::apply_solution;
use jj_lib::converge::propose_divergence_solution;
use jj_lib::files::FileMergeHunkLevel;
use jj_lib::merge::MergeBuilder;
use jj_lib::merge::SameChange;
use jj_lib::tree_merge::MergeOptions;
use pollster::FutureExt as _;

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
    /// The maximum number of evolog nodes to traverse, before giving up.
    #[arg(long, default_value_t = 100)]
    max_evolution_nodes: usize,
}

// TODO: consider adding logic to deal with more than one divergent change-id in
// one invocation.
pub(crate) fn cmd_converge(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConvergeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let converge_revset = workspace_command
        .settings()
        .get_string("revsets.converge")?;
    let divergent_change_search_space = workspace_command
        .parse_revset(ui, &RevisionArg::from(converge_revset))?
        .resolve()?;

    workspace_command.check_rewritable_expr(&divergent_change_search_space)?;

    let mut tx = workspace_command.start_transaction();
    let converge_ui = ConvergeCommandUI { ui };

    let solution = match propose_divergence_solution(
        tx.base_repo(),
        &converge_ui,
        divergent_change_search_space,
        args.max_evolution_nodes,
    )
    .block_on()
    {
        Ok(solution) => solution,
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
        "Resolving divergence on change {:.12}",
        solution.change_id.reverse_hex()
    )?;

    let (_new_commit, _num_rebased) = apply_solution(solution, tx.repo_mut())?;

    // TODO: provide a good transaction description.
    // TODO: return some summary information.
    tx.finish(ui, "converge commits")?;

    Ok(())
}

struct ConvergeCommandUI<'a> {
    ui: &'a mut Ui,
}

impl ConvergeUI for ConvergeCommandUI<'_> {
    fn choose_change<'a>(
        &self,
        divergent_changes: &'a CommitsByChangeId,
    ) -> Result<&'a ChangeId, ConvergeError> {
        if divergent_changes.is_empty() {
            return Err(ConvergeError::NoDivergentChanges());
        }

        if divergent_changes.len() == 1 {
            let (change_id, commits) = divergent_changes.iter().next().unwrap();
            // TODO: this output information seems to be in the wrong place. We probably
            // should output stuff even if there are multiple divergent changes.
            // TODO: list the commit ids and change-id in a user friendly way, consider
            // using {write,format}_commit_summary() instead of just displaying CommitId
            let mut it = commits.iter();
            let mut commit_ids = it
                .by_ref()
                .take(10)
                .map(|commit| format!("{:.12}", commit.id()))
                .join(", ");
            if it.next().is_some() {
                commit_ids.push_str(", ...");
            }
            writeln!(
                self.ui.status(),
                "Found one divergent change: {:.12} with {} commits: {}",
                change_id.reverse_hex(),
                commits.len(),
                commit_ids,
            )?;
            return Ok(change_id);
        }

        writeln!(self.ui.status(), "Multiple divergent changes found:")?;

        let mut formatter = self.ui.stderr_formatter();
        let mut choices: Vec<String> = Default::default();
        let change_ids: Vec<&ChangeId> = divergent_changes.keys().collect();
        for (i, change_id) in change_ids.iter().enumerate() {
            writeln!(formatter, "{}: {:.12}", i + 1, change_id.reverse_hex())?;
            choices.push(format!("{}", i + 1));
        }
        writeln!(formatter, "q: abort")?;
        choices.push("q".to_string());
        drop(formatter);
        let index =
            self.ui
                .prompt_choice("Enter the index of the change to converge", &choices, None)?;
        if index > change_ids.len() {
            writeln!(self.ui.status(), "No change selected, aborting...")?;
            Err(ConvergeError::UserAborted())
        } else {
            Ok(change_ids[index])
        }
    }

    fn choose_author(
        &self,
        divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<Signature, ConvergeError> {
        let mut formatter = self.ui.stderr_formatter();
        let mut choices: Vec<String> = Default::default();

        {
            let first_commit_author = divergent_commits[0].author();
            let mut all_same_author = true;
            for (i, commit) in divergent_commits.iter().enumerate() {
                writeln!(formatter, "{}: {:.12}", i + 1, commit.id())?;
                choices.push(format!("{}", i + 1));
                if commit.author() != first_commit_author {
                    all_same_author = false;
                }
            }
            if all_same_author {
                return Ok(first_commit_author.clone());
            }
        }

        writeln!(formatter, "q: abort")?;
        choices.push("q".to_string());
        drop(formatter);
        let index = self.ui.prompt_choice(
            "Enter the index of one of the divergent commits, its author will be the author of \
             the solution:",
            &choices,
            None,
        )?;
        if index > choices.len() {
            writeln!(self.ui.status(), "No commit selected, aborting...")?;
            Err(ConvergeError::UserAborted())
        } else {
            Ok(divergent_commits[index].author().clone())
        }
    }

    fn choose_parents(&self, divergent_commits: &[Commit]) -> Result<Vec<CommitId>, ConvergeError> {
        let mut formatter = self.ui.stderr_formatter();
        let mut choices: Vec<String> = Default::default();

        {
            let first_commit_parents = divergent_commits[0].parent_ids();
            let mut all_same_parents = true;
            for (i, commit) in divergent_commits.iter().enumerate() {
                writeln!(formatter, "{}: {:.12}", i + 1, commit.id())?;
                choices.push(format!("{}", i + 1));
                if commit.parent_ids() != first_commit_parents {
                    all_same_parents = false;
                }
            }
            if all_same_parents {
                return Ok(first_commit_parents.to_vec());
            }
        }

        writeln!(formatter, "q: abort")?;
        choices.push("q".to_string());
        drop(formatter);
        let index = self.ui.prompt_choice(
            "Enter the index of one of the divergent commits, its parents will be the parents of \
             the solution:",
            &choices,
            None,
        )?;
        if index > choices.len() {
            writeln!(self.ui.status(), "No commit selected, aborting...")?;
            Err(ConvergeError::UserAborted())
        } else {
            Ok(divergent_commits[index].parent_ids().to_vec())
        }
    }

    // TODO: Run the user's configured merge tool.
    fn merge_description(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<String, ConvergeError> {
        let description_merge = {
            let base = evolution_fork_point.description();
            let mut merge_builder = MergeBuilder::default();
            merge_builder.extend([base.to_string()]);
            for commit in divergent_commits {
                merge_builder.extend([commit.description().to_string(), base.to_string()]);
            }
            merge_builder.build()
        };
        let options = ConflictMaterializeOptions {
            marker_style: ConflictMarkerStyle::Diff,
            marker_len: None,
            merge: MergeOptions {
                hunk_level: FileMergeHunkLevel::Line,
                same_change: SameChange::Accept,
            },
        };
        Ok(materialize_merge_result_to_bytes(
            &description_merge,
            &ConflictLabels::unlabeled(),
            &options,
        )
        .to_string())
    }
}
