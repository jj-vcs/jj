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
use jj_lib::backend::TreeValue;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::materialize_tree_value;
use jj_lib::merge::Merge;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
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
    #[arg(long)]
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
    let repo = workspace_command.repo().clone();
    let tree = commit.tree();

    // Read the path's current tree value to determine if it's a file and to
    // preserve the executable bit and copy ID.
    let value = tree.path_value(&repo_path).await?;
    if value.is_tree() {
        let ui_path = workspace_command.format_file_path(&repo_path);
        return Err(user_error(format!("Path is a directory: {ui_path}")));
    }

    // Preserve metadata from the existing file entry if present.
    let (executable, copy_id) = if value.is_absent() {
        // New file: use defaults.
        (false, CopyId::placeholder())
    } else {
        let materialized =
            materialize_tree_value(repo.store(), &repo_path, value, tree.labels()).await?;
        match materialized {
            MaterializedTreeValue::File(file) => (file.executable, file.copy_id),
            MaterializedTreeValue::FileConflict(file) => (
                file.executable.unwrap_or(false),
                file.copy_id.unwrap_or_else(CopyId::placeholder),
            ),
            MaterializedTreeValue::Symlink { .. } | MaterializedTreeValue::GitSubmodule(_) => {
                let ui_path = workspace_command.format_file_path(&repo_path);
                return Err(user_error(format!(
                    "Path '{ui_path}' is not a regular file"
                )));
            }
            MaterializedTreeValue::OtherConflict { .. } => (false, CopyId::placeholder()),
            MaterializedTreeValue::AccessDenied(err) => {
                let ui_path = workspace_command.format_file_path(&repo_path);
                return Err(user_error(format!(
                    "Path '{ui_path}' exists but access is denied: {err}"
                )));
            }
            MaterializedTreeValue::Absent => {
                panic!("absent value was already checked above")
            }
            MaterializedTreeValue::Tree(_) => {
                panic!("tree value was already checked above")
            }
        }
    };

    if !args.stdin {
        return Err(user_error(
            "Pass `--stdin` to read the file contents from standard input",
        ));
    }
    let mut new_bytes: Vec<u8> = Vec::new();
    io::stdin().read_to_end(&mut new_bytes)?;

    let ui_path = workspace_command.format_file_path(&repo_path);
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
    let num_rebased = tx.repo_mut().rebase_descendants().await?;
    if let Some(mut formatter) = ui.status_formatter()
        && num_rebased > 0
    {
        writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
    }
    tx.finish(
        ui,
        format!("set file {} in commit {}", ui_path, commit.id().hex()),
    )
    .await?;
    Ok(())
}
