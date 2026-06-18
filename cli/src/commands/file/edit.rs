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
use jj_lib::backend::CopyId;
use jj_lib::backend::FileId;
use jj_lib::backend::TreeValue;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::choose_materialized_conflict_marker_len;
use jj_lib::conflicts::materialize_merge_result_to_bytes;
use jj_lib::conflicts::materialize_tree_value;
use jj_lib::conflicts::update_from_content;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
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

/// Edit the contents of a file in a revision
///
/// The file is opened with the contents from the given revision. After the
/// editor exits, the file is saved back to the revision and descendants are
/// rebased on top of the updated commit.
///
/// If the file does not yet exist in the revision, a new file will be created.
///
/// If the file is conflicted, the conflict markers are materialized in the
/// editor. Editing the conflict markers can resolve the conflict.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileEditArgs {
    /// The revision to edit the file in
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    revision: RevisionArg,

    /// Preserve the content (not the diff) when rebasing descendants
    #[arg(long)]
    restore_descendants: bool,

    /// The file to edit
    #[arg(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    path: String,
}

/// What kind of file we opened in the editor, plus the metadata needed to
/// write the result back.
enum EditTarget {
    /// A regular (non-conflicted) file.
    RegularFile { executable: bool, copy_id: CopyId },
    /// A conflicted file. We materialize the markers, let the user edit, then
    /// re-parse with `update_from_content`.
    ConflictedFile {
        marker_len: usize,
        unsimplified_ids: Merge<Option<FileId>>,
        /// The original `Merge<Option<TreeValue>>` — needed for
        /// `with_new_file_ids` when the conflict is not fully resolved.
        original_value: MergedTreeValue,
        executable: Option<bool>,
    },
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_edit(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileEditArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let commit = workspace_command
        .resolve_single_rev(ui, &args.revision)
        .await?;
    workspace_command.check_rewritable([commit.id()]).await?;

    let repo_path = workspace_command.parse_file_path(&args.path)?;
    let repo = workspace_command.repo().clone();
    let tree = commit.tree();

    let value = tree.path_value(&repo_path).await?;
    if value.is_tree() {
        let ui_path = workspace_command.format_file_path(&repo_path);
        return Err(user_error(format!("Path is a directory: {ui_path}")));
    }

    // Keep a clone of the original Merge<Option<TreeValue>> before consuming it
    // in materialize_tree_value — needed for with_new_file_ids on still-conflicted
    // edits.
    let original_value = value.clone();

    let conflict_marker_style = workspace_command.env().conflict_marker_style();
    let materialized =
        materialize_tree_value(repo.store(), &repo_path, value, tree.labels()).await?;

    let (original_bytes, edit_target) = match materialized {
        MaterializedTreeValue::File(mut file) => {
            let bytes = file.read_all(&repo_path).await?;
            let target = EditTarget::RegularFile {
                executable: file.executable,
                copy_id: file.copy_id,
            };
            (bytes, target)
        }
        MaterializedTreeValue::FileConflict(file) => {
            let marker_len = choose_materialized_conflict_marker_len(&file.contents);
            let options = ConflictMaterializeOptions {
                marker_style: conflict_marker_style,
                marker_len: Some(marker_len),
                merge: repo.store().merge_options().clone(),
            };
            let bytes = Vec::from(materialize_merge_result_to_bytes(
                &file.contents,
                &file.labels,
                &options,
            ));
            let target = EditTarget::ConflictedFile {
                marker_len,
                unsimplified_ids: file.unsimplified_ids,
                original_value,
                executable: file.executable,
            };
            (bytes, target)
        }
        MaterializedTreeValue::OtherConflict { .. } => {
            let ui_path = workspace_command.format_file_path(&repo_path);
            return Err(user_error(format!(
                "Path '{ui_path}' has a non-file conflict and cannot be edited"
            )));
        }
        MaterializedTreeValue::Symlink { .. } | MaterializedTreeValue::GitSubmodule(_) => {
            let ui_path = workspace_command.format_file_path(&repo_path);
            return Err(user_error(format!(
                "Path '{ui_path}' is not a regular file"
            )));
        }
        MaterializedTreeValue::AccessDenied(err) => {
            let ui_path = workspace_command.format_file_path(&repo_path);
            return Err(user_error(format!(
                "Path '{ui_path}' exists but access is denied: {err}"
            )));
        }
        MaterializedTreeValue::Absent => {
            // New file: open with empty content.
            (
                Vec::new(),
                EditTarget::RegularFile {
                    executable: false,
                    copy_id: CopyId::placeholder(),
                },
            )
        }
        MaterializedTreeValue::Tree(_) => {
            panic!("tree value was already checked above")
        }
    };

    let editor = workspace_command.text_editor()?;

    // Create a temp directory and place the file inside it using its exact
    // basename so editors can use the name for syntax highlighting and language
    // detection.
    let basename = repo_path
        .split()
        .map_or("file", |(_, basename)| basename.as_internal_str());

    let new_bytes = editor.edit_bytes(&original_bytes, Some(format!("_{basename}").as_ref()))?;

    if new_bytes == original_bytes {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    // Compute the new MergedTreeValue. For conflicts, re-parse the markers so
    // that a fully-resolved edit resolves the conflict and a partial edit
    // updates the conflict sides.
    let new_tree_value: MergedTreeValue = match edit_target {
        EditTarget::RegularFile {
            executable,
            copy_id,
        } => {
            let new_file_id = repo
                .store()
                .write_file(&repo_path, &mut new_bytes.as_slice())
                .await?;
            Merge::normal(TreeValue::File {
                id: new_file_id,
                executable,
                copy_id,
            })
        }
        EditTarget::ConflictedFile {
            marker_len,
            unsimplified_ids,
            original_value,
            executable,
        } => {
            let new_file_ids = update_from_content(
                &unsimplified_ids,
                repo.store(),
                &repo_path,
                &new_bytes,
                marker_len,
            )
            .await?;
            match new_file_ids.into_resolved() {
                Ok(file_id) => {
                    // Conflict fully resolved; preserve the executable bit.
                    let executable = executable.unwrap_or(false);
                    Merge::resolved(file_id.map(|id| TreeValue::File {
                        id,
                        executable,
                        copy_id: CopyId::placeholder(),
                    }))
                }
                Err(file_ids) => {
                    // Conflict still present; update only the file IDs so that
                    // executable bits and other metadata are preserved.
                    original_value.with_new_file_ids(&file_ids)
                }
            }
        }
    };

    let mut tree_builder = MergedTreeBuilder::new(commit.tree());
    tree_builder.set_or_remove(repo_path.clone(), new_tree_value);
    let new_tree = tree_builder.write_tree().await?;

    if new_tree.tree_ids() == commit.tree().tree_ids() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let ui_path = workspace_command.format_file_path(&repo_path);
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
    tx.finish(
        ui,
        format!("edit file {} in commit {}", ui_path, commit.id().hex()),
    )
    .await?;
    Ok(())
}
