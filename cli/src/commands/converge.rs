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

use std::io;

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
use jj_lib::converge::ConvergeResult;
use jj_lib::converge::ConvergeUI;
use jj_lib::converge::apply_solution;
use jj_lib::converge::choose_change;
use jj_lib::converge::converge_change;
use jj_lib::converge::find_divergent_changes;
use jj_lib::files::FileMergeHunkLevel;
use jj_lib::merge::MergeBuilder;
use jj_lib::merge::SameChange;
use jj_lib::tree_merge::MergeOptions;
use pollster::FutureExt as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::short_change_hash;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::CommandErrorKind;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Resolves divergent changes.
///
/// Attempts to resolve divergence by replacing the visible commits for a given
/// divergent change-id with a single commit.
///
/// See <https://github.com/jj-vcs/jj/blob/main/docs/design/jj-converge-command.md> for more details.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct ConvergeArgs {
    /// The search space to look for divergent commits (default: the
    /// 'revsets.converge' revset from config settings).
    #[arg(long, short, value_name = "REVSET", hide_default_value = true)]
    search_space: Option<RevisionArg>,

    /// In interactive mode, the user may be prompted to help resolve
    /// divergence (default: true).
    #[arg(long, short, default_value = "true")]
    interactive: bool,

    /// If true (the default), the divergent commits are replaced by a single
    /// solution commit. Otherwise, the solution commit is created as a hidden
    /// commit but the divergent commits are left unchanged (this
    /// can be useful for manually inspecting the solution).
    #[arg(long, default_value = "true", hide = true)]
    rewrite_divergent_commits: bool,

    /// The maximum number of evolog nodes to traverse, before giving up.
    #[arg(long, default_value_t = 100)]
    max_evolution_nodes: usize,
}

// TODO: consider adding logic to deal with more than one divergent change-id in
// one invocation. Pick one, solve it, pick another one, solve it, etc.
pub(crate) fn cmd_converge(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConvergeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let default_search_space = RevisionArg::from(
        workspace_command
            .settings()
            .get_string("revsets.converge")?,
    );
    let search_space = workspace_command
        .parse_revset(
            ui,
            args.search_space.as_ref().unwrap_or(&default_search_space),
        )?
        .resolve()?;

    workspace_command.check_rewritable_expr(&search_space)?;

    let mut tx = workspace_command.start_transaction();
    let repo = tx.base_repo();
    let divergent_changes = find_divergent_changes(repo, search_space)?;
    if divergent_changes.is_empty() {
        writeln!(
            ui.status(),
            "No divergent changes found in the specified revset."
        )?;
        return Ok(());
    }
    report_divergent_changes(ui, &divergent_changes, &tx.commit_summary_template())?;

    let solution = {
        let converge_ui: &dyn ConvergeUI = &ConvergeCommandUI { ui };
        let converge_ui = if args.interactive {
            Some(converge_ui)
        } else {
            None
        };

        let Some(change_id) = choose_change(converge_ui, &divergent_changes)? else {
            return Err(CommandError::new(
                CommandErrorKind::User,
                "No change selected",
            ));
        };

        // Note: change_id is one of the keys in divergent_changes, so this unwrap
        // should never fail.
        let divergent_commits: Vec<_> = divergent_changes
            .get(change_id)
            .unwrap()
            .values()
            .cloned()
            .collect();

        let solution_future = converge_change(
            repo,
            converge_ui,
            divergent_commits.as_slice(),
            args.max_evolution_nodes,
        );
        match solution_future.block_on()? {
            ConvergeResult::Solution(solution) => solution,
            ConvergeResult::NeedUserInput(msg) => {
                return Err(CommandError::new(
                    CommandErrorKind::Internal,
                    format!("Unexpected error during interactive converge: {msg}"),
                ));
            }
            ConvergeResult::Aborted => {
                return Err(CommandError::new(CommandErrorKind::User, "User aborted"));
            }
        }
    };

    let (solution_commit, num_rebased) =
        apply_solution(solution, args.rewrite_divergent_commits, tx.repo_mut())?;
    let transaction_description =
        report_progress(solution_commit, num_rebased, divergent_changes, ui, args)?;

    tx.finish(ui, transaction_description)?;
    Ok(())
}

struct ConvergeCommandUI<'a> {
    ui: &'a mut Ui,
}

impl ConvergeUI for ConvergeCommandUI<'_> {
    fn choose_change<'a>(
        &self,
        divergent_changes: &'a CommitsByChangeId,
    ) -> Result<Option<&'a ChangeId>, ConvergeError> {
        if divergent_changes.is_empty() {
            return Ok(None);
        }

        if divergent_changes.len() == 1 {
            let (change_id, _commits) = divergent_changes.iter().next().unwrap();
            return Ok(Some(change_id));
        }

        let mut formatter = self.ui.stderr_formatter();
        let mut choices: Vec<String> = Default::default();
        let change_ids: Vec<&ChangeId> = divergent_changes.keys().collect();
        for (i, change_id) in change_ids.iter().enumerate() {
            // TODO: is there a better way to display the change-id? perhaps with
            // format_short_change_id?
            writeln!(formatter, "{}: {}", i + 1, short_change_hash(change_id))?;
            choices.push(format!("{}", i + 1));
        }
        writeln!(formatter, "q: abort")?;
        choices.push("q".to_string());
        drop(formatter);
        let index =
            self.ui
                .prompt_choice("Enter the index of the change to converge", &choices, None)?;
        if index >= change_ids.len() {
            Ok(None)
        } else {
            Ok(Some(change_ids[index]))
        }
    }

    fn choose_author(
        &self,
        divergent_commits: &[Commit],
        _evolution_fork_point: &Commit,
    ) -> Result<Option<Signature>, ConvergeError> {
        self.choose(
            divergent_commits,
            |commit| commit.author().clone(),
            "Enter the index of one of the divergent commits, its author will be the author of \
             the solution:",
        )
    }

    fn choose_parents(
        &self,
        divergent_commits: &[Commit],
    ) -> Result<Option<Vec<CommitId>>, ConvergeError> {
        self.choose(
            divergent_commits,
            |commit| commit.parent_ids().to_vec(),
            "Enter the index of one of the divergent commits, its parent(s) will be the parents \
             of the solution:",
        )
    }

    // TODO: Run the user's configured merge tool.
    fn merge_description(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<Option<String>, ConvergeError> {
        // TODO: this probably needs more work. We should only show distinct
        // descriptions (i.e. we should dedup).
        let (description_merge, conflict_labels) = {
            let base = evolution_fork_point.description();
            let base_label = evolution_fork_point.conflict_label();
            let mut merge_builder = MergeBuilder::default();
            let mut labels = vec![];
            merge_builder.extend([base.to_string()]);
            labels.push(base_label.clone());
            for commit in divergent_commits {
                merge_builder.extend([commit.description().to_string(), base.to_string()]);
                labels.extend([commit.conflict_label(), base_label.clone()]);
            }
            (merge_builder.build(), ConflictLabels::from_vec(labels))
        };
        let options = ConflictMaterializeOptions {
            marker_style: ConflictMarkerStyle::Diff,
            marker_len: None,
            merge: MergeOptions {
                hunk_level: FileMergeHunkLevel::Line,
                same_change: SameChange::Accept,
            },
        };
        Ok(Some(
            materialize_merge_result_to_bytes(&description_merge, &conflict_labels, &options)
                .to_string(),
        ))
    }
}

impl ConvergeCommandUI<'_> {
    fn choose<T>(
        &self,
        divergent_commits: &[Commit],
        value_fn: fn(&Commit) -> T,
        prompt: &str,
    ) -> Result<Option<T>, ConvergeError>
    where
        T: PartialEq + Clone,
    {
        let mut formatter = self.ui.stderr_formatter();
        let mut choices: Vec<String> = Default::default();

        {
            let first_commit_value = value_fn(&divergent_commits[0]);
            let mut all_same_value = true;
            for (i, commit) in divergent_commits.iter().enumerate() {
                // TODO: is there a better way to display the commit-id?
                writeln!(formatter, "{}: {}", i + 1, short_commit_hash(commit.id()))?;
                choices.push(format!("{}", i + 1));
                if value_fn(commit) != first_commit_value {
                    all_same_value = false;
                }
            }
            if all_same_value {
                return Ok(Some(first_commit_value));
            }
        }

        writeln!(formatter, "q: abort")?;
        choices.push("q".to_string());
        drop(formatter);
        let index = self.ui.prompt_choice(prompt, &choices, None)?;
        if index >= divergent_commits.len() {
            Ok(None)
        } else {
            Ok(Some(value_fn(&divergent_commits[index])))
        }
    }
}

fn report_divergent_changes(
    ui: &Ui,
    divergent_changes: &CommitsByChangeId,
    commit_summary_template: &TemplateRenderer<Commit>,
) -> io::Result<()> {
    let mut formatter = ui.stdout_formatter();
    writeln!(
        ui.status(),
        "Found {} divergent changes in the specified revset:",
        divergent_changes.len()
    )?;
    for (change_id, commits) in divergent_changes {
        writeln!(
            ui.status(),
            "- Change: {} with {} commits:",
            short_change_hash(change_id),
            commits.len(),
        )?;
        let it = commits.iter();
        for (_, commit) in it.take(10) {
            write!(formatter, "    ")?;
            commit_summary_template.format(commit, formatter.as_mut())?;
            writeln!(formatter)?;
        }
        if commits.len() > 10 {
            write!(formatter, "    ... and {} more", commits.len() - 10)?;
        }
        writeln!(formatter)?;
    }
    Ok(())
}

fn report_progress(
    solution_commit: Commit,
    num_rebased: usize,
    divergent_changes: CommitsByChangeId,
    ui: &Ui,
    args: &ConvergeArgs,
) -> Result<String, io::Error> {
    let transaction_description = if args.rewrite_divergent_commits {
        format!(
            "converge {}",
            short_change_hash(solution_commit.change_id())
        )
    } else {
        format!(
            "converge {} (rewrite_divergent_commits=false), created commit {}",
            short_change_hash(solution_commit.change_id()),
            short_commit_hash(solution_commit.id())
        )
    }
    .clone();

    if args.rewrite_divergent_commits {
        write!(
            ui.status(),
            "Successfully converged change: created commit {}.",
            short_commit_hash(solution_commit.id()),
        )?;
        if num_rebased > 0 {
            write!(ui.status(), "Rebased {num_rebased} descendants")?;
        }
        writeln!(ui.status())?;
    } else {
        writeln!(
            ui.status(),
            "Created a hidden solution commit {} for change {}, but left divergent commits \
             unchanged. You can inspect the solution commit, e.g. to compare it against the \
             divergent commits.",
            short_commit_hash(solution_commit.id()),
            short_change_hash(solution_commit.change_id()),
        )?;
    }

    if divergent_changes.len() > 1 {
        writeln!(
            ui.hint_default(),
            "There are still {} divergent changes remaining in the specified revset, you may want \
             to run this command again to resolve another one.",
            divergent_changes.len() - 1
        )?;
    }
    Ok(transaction_description)
}
