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

use std::io::Write as _;

use clap_complete::ArgValueCompleter;
use jj_lib::merge::Merge;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Delete files from the given revision
///
/// Removes all files matched by the given filesets from the specified revision.
/// Descendants are rebased on top of the rewritten commit.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileDeleteArgs {
    /// The revision to delete the files in
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    revision: RevisionArg,

    /// Preserve the content (not the diff) when rebasing descendants
    #[arg(long)]
    restore_descendants: bool,

    /// Files or directories to delete (filesets are accepted)
    #[arg(required = true, value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_delete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileDeleteArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let commit = workspace_command
        .resolve_single_rev(ui, &args.revision)
        .await?;
    workspace_command.check_rewritable([commit.id()]).await?;

    let tree = commit.tree();
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();
    print_unmatched_explicit_paths(ui, &workspace_command, &fileset_expression, [&tree])?;

    let mut tree_builder = MergedTreeBuilder::new(commit.tree());
    for (path, _value) in tree.entries_matching(matcher.as_ref()) {
        tree_builder.set_or_remove(path, Merge::absent());
    }
    let new_tree = tree_builder.write_tree().await?;

    if new_tree.tree_ids() == commit.tree().tree_ids() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let mut tx = workspace_command.start_transaction();
    tx.repo_mut()
        .rewrite_commit(&commit)
        .set_tree(new_tree)
        .write()
        .await?;
    let (num_rebased, extra_msg) = if args.restore_descendants {
        (
            tx.repo_mut().reparent_descendants().await?,
            " (while preserving their content)",
        )
    } else {
        (tx.repo_mut().rebase_descendants().await?, "")
    };
    if let Some(mut formatter) = ui.status_formatter()
        && num_rebased > 0
    {
        writeln!(
            formatter,
            "Rebased {num_rebased} descendant commits{extra_msg}"
        )?;
    }
    tx.finish(ui, format!("delete paths in commit {}", commit.id().hex()))
        .await?;
    Ok(())
}
