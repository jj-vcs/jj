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

use std::collections::HashSet;
use std::io::Write as _;

use itertools::Itertools;
use jj_lib::backend::CommitId;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::object_id::ObjectId as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Clears predecessors for the given revisions
///
/// This command creates new versions of the given revisions, with no
/// predecessors set. Older versions of the revisions will no longer be visible
/// in `jj evolog`.
///
/// It will also allow older versions of the revisions to be completely removed
/// from the repository after the operation introducing it is deleted and the
/// repository is garbage collected.
#[derive(clap::Args, Clone, Debug)]
pub struct UtilClearPredecessorsArgs {
    /// The revisions to clear predecessors for.
    #[arg(long, short, value_name = "REVSETS")]
    revisions: Vec<RevisionArg>,
}

pub fn cmd_util_clear_predecessors(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilClearPredecessorsArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let target_commits: Vec<_> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;
    if target_commits.is_empty() {
        writeln!(ui.status(), "No revisions to modify.")?;
        return Ok(());
    }
    let target_set: HashSet<&CommitId> = target_commits.iter().ids().collect();
    workspace_command.check_rewritable(target_set.iter().copied())?;

    let mut tx = workspace_command.start_transaction();
    let mut num_rebased = 0;
    tx.repo_mut().transform_descendants(
        target_set.iter().copied().cloned().collect(),
        |rewriter| {
            if target_set.contains(rewriter.old_commit().id()) {
                rewriter.reparent().set_predecessors(vec![]).write()?;
            } else {
                rewriter.rebase()?.write()?;
                num_rebased += 1;
            }
            Ok(())
        },
    )?;
    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "Cleared predecessors for {} commits.",
            target_commits.len()
        )?;
        if num_rebased > 0 {
            writeln!(formatter, "Rebased {num_rebased} descendant commits",)?;
        }
    }
    let transaction_description = if target_commits.len() == 1 {
        format!(
            "cleared predecessors for commit {}",
            target_commits[0].id().hex()
        )
    } else {
        format!(
            "cleared predecessors for commit {} and {} more",
            target_commits[0].id().hex(),
            target_commits.len() - 1
        )
    };
    tx.finish(ui, transaction_description)?;
    Ok(())
}
