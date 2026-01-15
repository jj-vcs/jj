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

use clap_complete::ArgValueCompleter;
use jj_lib::local_working_copy::LockedLocalWorkingCopy;
use jj_lib::merge::Merge;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::export_working_copy_changes_to_git;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Stop tracking specified paths in the working copy
///
/// The untracked files will remain on disk but won't be included in commits.
/// They will stay untracked until you explicitly run `jj file track` on them.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileUntrackArgs {
    /// Paths to untrack
    #[arg(required = true, value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_file_untrack(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileUntrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    let working_copy_shared_with_git = workspace_command.working_copy_shared_with_git();

    let mut tx = workspace_command.start_transaction().into_inner();
    let (mut locked_ws, wc_commit) = workspace_command.start_working_copy_mutation()?;

    // Collect paths to untrack from the current tree
    let wc_tree = wc_commit.tree();
    let paths_to_untrack: Vec<_> = wc_tree
        .entries_matching(matcher.as_ref())
        .map(|(path, _)| path)
        .collect();

    // Create a new tree without the unwanted files
    let mut tree_builder = MergedTreeBuilder::new(wc_tree.clone());
    for path in &paths_to_untrack {
        tree_builder.set_or_remove(path.clone(), Merge::absent());
    }
    let new_tree = tree_builder.write_tree()?;
    let new_commit = tx
        .repo_mut()
        .rewrite_commit(&wc_commit)
        .set_tree(new_tree)
        .write()?;

    // Add paths to the persistent untracked list so they won't be
    // automatically tracked again on subsequent snapshots.
    if let Some(locked_local_wc) = locked_ws
        .locked_wc()
        .downcast_mut::<LockedLocalWorkingCopy>()
    {
        locked_local_wc.add_to_untracked(paths_to_untrack)?;
    }

    // Reset the working copy to the new commit
    locked_ws.locked_wc().reset(&new_commit).block_on()?;

    let num_rebased = tx.repo_mut().rebase_descendants()?;
    if num_rebased > 0 {
        writeln!(ui.status(), "Rebased {num_rebased} descendant commits")?;
    }
    if working_copy_shared_with_git {
        export_working_copy_changes_to_git(ui, tx.repo_mut(), &wc_tree, &new_commit.tree())?;
    }
    let repo = tx.commit("untrack paths")?;
    locked_ws.finish(repo.op_id().clone())?;
    print_unmatched_explicit_paths(ui, &workspace_command, &fileset_expression, [&wc_tree])?;
    Ok(())
}
