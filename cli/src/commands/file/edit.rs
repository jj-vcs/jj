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
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::backend::TreeValue;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::fsmonitor::FsmonitorSettings;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::local_working_copy::EolConversionMode;
use jj_lib::local_working_copy::ExecChangeSetting;
use jj_lib::local_working_copy::TreeState;
use jj_lib::local_working_copy::TreeStateSettings;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::repo::Repo as _;
use jj_lib::working_copy::SnapshotOptions;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::rebase_or_reparent_descendants;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::complete;
use crate::description_util::LineNumber;
use crate::description_util::TempTextEditError;
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
    let repo = workspace_command.repo();
    let tree = commit.tree();

    let value = tree.path_value(&repo_path).await?;
    let ui_path = workspace_command.format_file_path(&repo_path);

    // Validate that the path points at something we can edit as text
    let conflict_marker_style = workspace_command.env().conflict_marker_style();
    match value.into_resolved() {
        Ok(None | Some(TreeValue::File { .. })) => {}
        Ok(Some(TreeValue::Tree(_))) => {
            return Err(user_error(format!("Path is a directory: {ui_path}")));
        }
        Ok(Some(TreeValue::Symlink(_) | TreeValue::GitSubmodule(_))) => {
            return Err(user_error(format!(
                "Path '{ui_path}' is not a regular file"
            )));
        }
        Err(conflict) => {
            // A conflict is editable only if every term is a file (or absent).
            if conflict.to_file_merge().is_none() {
                return Err(user_error(format!(
                    "Path '{ui_path}' has a non-file conflict and cannot be edited"
                )));
            }
        }
    }

    // Materialize a sparse working copy in a temporary directory containing only
    // the file being edited, open the editor on it, then snapshot the result.
    let temp_dir = tempfile::Builder::new().prefix("jj-file-edit-").tempdir()?;
    let wc_path = temp_dir.path().join("wc");
    let state_dir = temp_dir.path().join("state");
    std::fs::create_dir(&wc_path).context(&wc_path)?;
    std::fs::create_dir(&state_dir).context(&state_dir)?;
    let tree_state_settings = TreeStateSettings {
        conflict_marker_style,
        eol_conversion_mode: EolConversionMode::None,
        exec_change_setting: ExecChangeSetting::Auto,
        fsmonitor_settings: FsmonitorSettings::None,
    };
    let mut tree_state = TreeState::init(
        repo.store().clone(),
        wc_path,
        state_dir,
        &tree_state_settings,
    )
    .map_err(user_error)?;
    tree_state
        .set_sparse_patterns(vec![repo_path.clone()])
        .map_err(internal_error)?;
    tree_state.check_out(&tree).map_err(internal_error)?;

    let file_path = repo_path
        .to_fs_path(tree_state.working_copy_path())
        .map_err(user_error)?;
    let editor = workspace_command.text_editor()?;
    if let Err(err) = editor.edit_file(&file_path, LineNumber::MIN) {
        // Keep the temporary working copy so the user can recover their edits.
        let _kept_dir = temp_dir.keep();
        return Err(TempTextEditError {
            error: Box::new(err),
            name: None,
            path: Some(file_path),
        }
        .into());
    }

    tree_state
        .snapshot(&SnapshotOptions {
            base_ignores: GitIgnoreFile::empty(),
            progress: None,
            start_tracking_matcher: &EverythingMatcher,
            force_tracking_matcher: &NothingMatcher,
            max_new_file_size: u64::MAX,
        })
        .await?;
    let new_tree = tree_state.current_tree().clone();

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
    rebase_or_reparent_descendants(ui, tx.repo_mut(), args.restore_descendants).await?;
    tx.finish(
        ui,
        format!(
            "edit file {} in commit {}",
            repo_path.as_internal_file_string(),
            commit.id()
        ),
    )
    .await?;
    Ok(())
}
