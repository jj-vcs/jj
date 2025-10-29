// Copyright 2020 The Jujutsu Authors
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

use std::io::Write as _;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::repo::Repo as _;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::revset::RevsetIteratorExt as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::ui::Ui;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MovementArgs {
    pub offset: u64,
    pub edit: bool,
    pub no_edit: bool,
    pub conflict: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MovementArgsInternal {
    offset: u64,
    should_edit: bool,
    conflict: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Direction {
    Next,
    Prev,
}

impl Direction {
    fn cmd(&self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Prev => "prev",
        }
    }

    fn target_not_found_error(
        &self,
        workspace_command: &WorkspaceCommandHelper,
        args: &MovementArgsInternal,
        commits: &[Commit],
    ) -> CommandError {
        let offset = args.offset;
        let err_msg = match (self, args.should_edit, args.conflict) {
            // in edit mode, start_revset is the WC, so we only look for direct descendants.
            (Self::Next, true, true) => {
                String::from("The working copy has no descendants with conflicts")
            }
            (Self::Next, true, false) => {
                format!("No descendant found {offset} commit(s) forward from the working copy",)
            }
            // in non-edit mode, start_revset is the parent of WC, so we look for other descendants
            // of start_revset.
            (Self::Next, false, true) => {
                String::from("The working copy parent(s) have no other descendants with conflicts")
            }
            (Self::Next, false, false) => format!(
                "No other descendant found {offset} commit(s) forward from the working copy \
                 parent(s)",
            ),
            // The WC can never be an ancestor of the start_revset since start_revset is either
            // itself or it's parent.
            (Self::Prev, true, true) => {
                String::from("The working copy has no ancestors with conflicts")
            }
            (Self::Prev, true, false) => {
                format!("No ancestor found {offset} commit(s) back from the working copy",)
            }
            (Self::Prev, false, true) => {
                String::from("The working copy parent(s) have no ancestors with conflicts")
            }
            (Self::Prev, false, false) => format!(
                "No ancestor found {offset} commit(s) back from the working copy parents(s)",
            ),
        };

        let template = workspace_command.commit_summary_template();
        let mut cmd_err = user_error(err_msg);
        for commit in commits {
            cmd_err.add_formatted_hint_with(|formatter| {
                if args.should_edit {
                    write!(formatter, "Working copy: ")?;
                } else {
                    write!(formatter, "Working copy parent: ")?;
                }
                template.format(commit, formatter)
            });
        }

        cmd_err
    }

    fn build_target_revset(
        &self,
        working_revset: &Arc<ResolvedRevsetExpression>,
        start_revset: &Arc<ResolvedRevsetExpression>,
        args: &MovementArgsInternal,
    ) -> Result<Arc<ResolvedRevsetExpression>, CommandError> {
        let nth = match (self, args.should_edit) {
            (Self::Next, true) => start_revset.descendants_at(args.offset),
            (Self::Next, false) => start_revset
                .children()
                .minus(working_revset)
                .descendants_at(args.offset - 1),
            (Self::Prev, _) => start_revset.ancestors_at(args.offset),
        };

        let target_revset = match (self, args.conflict) {
            (_, false) => nth,
            (Self::Next, true) => nth
                .descendants()
                .filtered(RevsetFilterPredicate::HasConflict)
                .roots(),
            // If people desire to move to the root conflict, replace the `heads()` below
            // with `roots(). But let's wait for feedback.
            (Self::Prev, true) => nth
                .ancestors()
                .filtered(RevsetFilterPredicate::HasConflict)
                .heads(),
        };

        Ok(target_revset)
    }
}

fn get_target_commit(
    ui: &mut Ui,
    workspace_command: &WorkspaceCommandHelper,
    direction: Direction,
    working_commit_id: &CommitId,
    args: &MovementArgsInternal,
) -> Result<Commit, CommandError> {
    let wc_revset = RevsetExpression::commit(working_commit_id.clone());

    // If we're not editing, the working-copy shouldn't have any children
    if !args.should_edit
        && !workspace_command
            .repo()
            .view()
            .heads()
            .contains(working_commit_id)
    {
        return Err(user_error_with_hint(
            "The working copy must not have any children",
            "Create a new commit on top of this one or use `--edit`",
        ));
    }

    // If we're editing, start at the working-copy commit. Otherwise, start from
    // its direct parent(s).
    let start_revset = if args.should_edit {
        wc_revset.clone()
    } else {
        wc_revset.parents()
    };

    let target_revset = direction.build_target_revset(&wc_revset, &start_revset, args)?;

    let targets: Vec<Commit> = target_revset
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .commits(workspace_command.repo().store())
        .try_collect()?;

    let target = match targets.as_slice() {
        [target] => target,
        [] => {
            // We found no ancestor/descendant.
            let start_commits: Vec<Commit> = start_revset
                .evaluate(workspace_command.repo().as_ref())?
                .iter()
                .commits(workspace_command.repo().store())
                .try_collect()?;
            return Err(direction.target_not_found_error(workspace_command, args, &start_commits));
        }
        commits => choose_commit(ui, workspace_command, direction, commits)?,
    };

    Ok(target.clone())
}

fn choose_commit<'a>(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    direction: Direction,
    commits: &'a [Commit],
) -> Result<&'a Commit, CommandError> {
    writeln!(
        ui.stderr(),
        "ambiguous {} commit, choose one to target:",
        direction.cmd()
    )?;
    let mut formatter = ui.stderr_formatter();
    let template = workspace_command.commit_summary_template();
    let mut choices: Vec<String> = Default::default();
    for (i, commit) in commits.iter().enumerate() {
        write!(formatter, "{}: ", i + 1)?;
        template.format(commit, formatter.as_mut())?;
        writeln!(formatter)?;
        choices.push(format!("{}", i + 1));
    }
    writeln!(formatter, "q: quit the prompt")?;
    choices.push("q".to_string());
    drop(formatter);

    let index = ui.prompt_choice(
        "enter the index of the commit you want to target",
        &choices,
        None,
    )?;
    commits
        .get(index)
        .ok_or_else(|| user_error("ambiguous target commit"))
}

pub(crate) fn move_to_commit(
    ui: &mut Ui,
    command: &CommandHelper,
    direction: Direction,
    args: &MovementArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let current_wc_id = workspace_command
        .get_wc_commit_id()
        .ok_or_else(|| user_error("This command requires a working copy"))?;

    let config_edit_flag = workspace_command.settings().get_bool("ui.movement.edit")?;
    let args = MovementArgsInternal {
        should_edit: args.edit || (!args.no_edit && config_edit_flag),
        offset: args.offset,
        conflict: args.conflict,
    };

    let target = get_target_commit(ui, &workspace_command, direction, current_wc_id, &args)?;
    let current_short = short_commit_hash(current_wc_id);
    let target_short = short_commit_hash(target.id());
    let cmd = direction.cmd();
    // We're editing, just move to the target commit.
    if args.should_edit {
        // We're editing, the target must be rewritable.
        workspace_command.check_rewritable([target.id()])?;
        let mut tx = workspace_command.start_transaction();
        tx.edit(&target)?;
        tx.finish(
            ui,
            format!("{cmd}: {current_short} -> editing {target_short}"),
        )?;
        return Ok(());
    }
    let mut tx = workspace_command.start_transaction();
    // Move the working-copy commit to the new parent.
    tx.check_out(&target)?;
    tx.finish(ui, format!("{cmd}: {current_short} -> {target_short}"))?;
    Ok(())
}
