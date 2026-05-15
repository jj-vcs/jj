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
use std::io::Read as _;
use std::io::Write as _;

use clap_complete::ArgValueCompleter;
use jj_lib::backend::CopyId;
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts::resolve_file_executable;
use jj_lib::merge::Merge;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::rebase_or_reparent_descendants;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Update the contents of a file in the given revision
///
/// The new file contents must be provided via `--stdin`. In the future,
/// additional content sources may be supported.
///
/// Descendants are rebased on top of the rewritten commit.
///
/// Example usage:
///
/// ```shell
/// echo "new file contents" | jj file set --stdin -r xyz path/to/file.md
/// ```
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileSetArgs {
    /// The revision to set the file in
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    revision: RevisionArg,

    /// Read the new file contents from standard input
    #[arg(long, required = true)]
    stdin: bool,

    /// The file to set
    #[arg(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    path: String,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileSetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let commit = workspace_command
        .resolve_single_rev(ui, &args.revision)
        .await?;
    workspace_command.check_rewritable([commit.id()]).await?;

    let repo_path = workspace_command.parse_file_path(&args.path)?;
    let repo = workspace_command.repo();
    let tree = commit.tree();

    let ui_path = workspace_command.format_file_path(&repo_path);

    // Read the path's current tree value to determine if it's a file and to
    // preserve the executable bit and copy ID.
    let value = tree.path_value(&repo_path).await?;
    if value.is_tree() {
        return Err(user_error(format!("Path is a directory: {ui_path}")));
    }
    let (executable, copy_id) = match value.into_resolved() {
        // New file: use defaults.
        Ok(None) => (false, CopyId::placeholder()),
        Ok(Some(TreeValue::File {
            id: _,
            executable,
            copy_id,
        })) => (executable, copy_id),
        Ok(Some(TreeValue::Symlink(_) | TreeValue::GitSubmodule(_))) => {
            return Err(user_error(format!(
                "Path '{ui_path}' is not a regular file"
            )));
        }
        Ok(Some(TreeValue::Tree(_))) => unreachable!("tree value was already checked above"),
        // The conflict is replaced by the new contents. Keep the executable bit
        // if the conflict sides agree on it.
        Err(conflict) => {
            let executable = conflict
                .to_executable_merge()
                .and_then(|merge| resolve_file_executable(&merge))
                .unwrap_or(false);
            (executable, CopyId::placeholder())
        }
    };

    let mut new_bytes: Vec<u8> = Vec::new();
    io::stdin().read_to_end(&mut new_bytes)?;

    let new_file_id = repo
        .store()
        .write_file(&repo_path, &mut new_bytes.as_slice())
        .await?;
    let new_tree_value = Merge::normal(TreeValue::File {
        id: new_file_id,
        executable,
        copy_id,
    });
    let mut tree_builder = MergedTreeBuilder::new(commit.tree());
    tree_builder.set_or_remove(repo_path.clone(), new_tree_value);
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
    rebase_or_reparent_descendants(ui, tx.repo_mut(), false).await?;
    tx.finish(
        ui,
        format!(
            "set file {} in commit {}",
            repo_path.as_internal_file_string(),
            commit.id().hex()
        ),
    )
    .await?;
    Ok(())
}
