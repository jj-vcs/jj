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

use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::op_store::View;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::workspace::Workspace;
use jj_lib::workspace_store::SimpleWorkspaceStore;
use jj_lib::workspace_store::WorkspaceStore as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
#[cfg(feature = "git")]
use crate::commands::git::is_push_operation;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::view_with_desired_portions_restored;
use crate::ui::Ui;

/// Undo the last operation
///
/// If used once after a normal (non-`undo`) operation, this will undo that last
/// operation by restoring its parent. If `jj undo` is used repeatedly, it will
/// restore increasingly older operations, going further back into the past.
///
/// There is also a complementary `jj redo` command that would instead move in
/// the direction of the future after one or more `jj undo`s.
///
/// Use `jj op log` to visualize the log of past operations, including a
/// detailed description of any past undo/redo operations. See also `jj op
/// restore` to explicitly restore an older operation by its id (available in
/// the operation log).
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {}

pub(crate) const UNDO_OP_DESC_PREFIX: &str = "undo: restore to operation ";
pub(crate) const WORKSPACE_DELETE_OP_DESC_PREFIX: &str = "delete workspace";

pub async fn cmd_undo(
    ui: &mut Ui,
    command: &CommandHelper,
    _: &UndoArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;

    let mut target_op = workspace_command.repo().operation().clone();

    // Growing the "undo-stack" works as follows. See also the
    // [redo-stack](./redo.rs), which works in a similar way.
    //
    // - If the operation to undo is a regular one (not an undo-operation), simply
    //   undo it (== restore its parent).
    // - If the operation to undo is an undo-operation itself, undo that operation
    //   to which the previous undo-operation restored the repo.
    // - If the operation to restore to is an undo-operation, restore directly to
    //   the original operation. This avoids creating a linked list of
    //   undo-operations, which subsequently may have to be walked with an
    //   inefficient loop.
    //
    // This described behavior leads to "jumping over" old undo-stacks if the
    // current one grows into it. Consider this op-log example:
    //
    // * G "undo: restore A" -------+
    // |                            |
    // * F "undo: restore B" -----+ |
    // |                          | |
    // * E                        | |
    // |                          | |
    // * D "undo: restore B" -+   | |
    // |                      |   | |
    // * C                    |   | |
    // |                      |   | |
    // * B   <----------------+ <-+ |
    // |                            |
    // * A   <----------------------+
    //
    // It was produced by the following sequence of events:
    // - do normal operations A, B and C
    // - undo C, restoring to B
    // - do normal operation E
    // - undo E, restoring to B again (NOT to D)
    // - undo F, restoring to A
    //
    // Notice that running `undo` after having undone E leads to A being
    // restored (as opposed to C). The undo-stack spanning from F to B was
    // "jumped over".
    //
    if let Some(target_op_hex) = target_op
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        let target_op_id = OperationId::try_from_hex(target_op_hex).ok_or_else(|| {
            internal_error("Failed to parse ID of target operation in undo-stack")
        })?;
        target_op = workspace_command
            .repo()
            .loader()
            .load_operation(&target_op_id)
            .await?;
    }
    #[cfg(feature = "git")]
    if is_push_operation(&target_op) {
        writeln!(
            ui.warning_default(),
            "Undoing a push operation often leads to conflicted bookmarks."
        )?;
        writeln!(ui.hint_default(), "To avoid this, run `jj redo` now.")?;
    }

    let mut target_op_parent = match target_op.parents().await?.into_iter().at_most_one() {
        Ok(Some(op)) => op,
        Ok(None) => return Err(user_error("Cannot undo root operation")),
        Err(_) => {
            return Err(user_error("Cannot undo a merge operation")
                .hinted("Consider using `jj op restore` instead"));
        }
    };

    // Avoid the creation of a linked list by restoring to the original
    // operation directly, if we're about to restore an undo-operation. If we
    // didn't do this, repeated calls of `jj new ; jj undo` would create an
    // ever-growing linked list of undo-operations that restore each other.
    // Calling `jj undo` one more time would have to restore to the operation
    // at the very beginning of the linked list, which would require walking the
    // entire thing unnecessarily.
    if let Some(target_op_parent_hex) = target_op_parent
        .metadata()
        .description
        .strip_prefix(UNDO_OP_DESC_PREFIX)
    {
        let target_op_parent_id =
            OperationId::try_from_hex(target_op_parent_hex).ok_or_else(|| {
                internal_error("Failed to parse ID of target operation's parent in undo-stack")
            })?;
        target_op_parent = workspace_command
            .repo()
            .loader()
            .load_operation(&target_op_parent_id)
            .await?;
    }

    let old_view = workspace_command.repo().view().store_view().clone();
    let restored_view = target_op_parent.view().await?.store_view().clone();
    let restores_workspace_delete = target_op
        .metadata()
        .description
        .starts_with(WORKSPACE_DELETE_OP_DESC_PREFIX);
    if restores_workspace_delete {
        check_workspace_restore_directories_available(
            &workspace_command,
            &old_view,
            &restored_view,
        )?;
    }
    let mut tx = workspace_command.start_transaction();
    let new_view =
        view_with_desired_portions_restored(&restored_view, &old_view, &DEFAULT_REVERT_WHAT);
    tx.repo_mut().set_view(new_view);
    if let Some(mut formatter) = ui.status_formatter() {
        let template = tx.base_workspace_helper().operation_summary_template();

        write!(formatter, "Undid operation: ")?;
        template.format(&target_op, formatter.as_mut())?;
        writeln!(formatter)?;

        write!(formatter, "Restored to operation: ")?;
        template.format(&target_op_parent, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(
        ui,
        format!("{UNDO_OP_DESC_PREFIX}{}", target_op_parent.id().hex()),
    )
    .await?;

    if restores_workspace_delete {
        restore_workspace_directories(ui, command, &workspace_command, &old_view, &restored_view)
            .await?;
    }

    Ok(())
}

pub(crate) async fn restore_workspace_directories(
    ui: &mut Ui,
    command: &CommandHelper,
    workspace_command: &WorkspaceCommandHelper,
    old_view: &View,
    new_view: &View,
) -> Result<(), CommandError> {
    let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;
    let working_copy_factory = command.get_working_copy_factory()?;
    for workspace_name in new_view
        .wc_commit_ids
        .keys()
        .filter(|workspace_name| !old_view.wc_commit_ids.contains_key(*workspace_name))
        .cloned()
        .collect_vec()
    {
        let Some(rel_path) = workspace_store.get_workspace_path(&workspace_name)? else {
            continue;
        };
        let workspace_path = normalize_maybe_missing(workspace_command.repo_path().join(rel_path))?;
        if workspace_path.exists() {
            return Err(workspace_restore_blocked_error(
                &workspace_name,
                &workspace_path,
            ));
        }
        let Some(wc_commit_id) = workspace_command
            .repo()
            .view()
            .get_wc_commit_id(&workspace_name)
        else {
            continue;
        };
        let wc_commit = workspace_command
            .repo()
            .store()
            .get_commit_async(wc_commit_id)
            .await?;
        Workspace::restore_workspace_with_existing_repo(
            &workspace_path,
            workspace_command.repo_path(),
            workspace_command.repo(),
            working_copy_factory,
            workspace_name,
            &wc_commit,
        )
        .await?;
        writeln!(
            ui.status(),
            r#"Restored workspace directory "{}"."#,
            workspace_path.display()
        )?;
    }
    Ok(())
}

fn check_workspace_restore_directories_available(
    workspace_command: &WorkspaceCommandHelper,
    old_view: &View,
    new_view: &View,
) -> Result<(), CommandError> {
    let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;
    for workspace_name in new_view
        .wc_commit_ids
        .keys()
        .filter(|workspace_name| !old_view.wc_commit_ids.contains_key(*workspace_name))
        .cloned()
        .collect_vec()
    {
        let Some(rel_path) = workspace_store.get_workspace_path(&workspace_name)? else {
            continue;
        };
        let workspace_path = normalize_maybe_missing(workspace_command.repo_path().join(rel_path))?;
        if workspace_path.exists() {
            return Err(workspace_restore_blocked_error(
                &workspace_name,
                &workspace_path,
            ));
        }
    }
    Ok(())
}

fn workspace_restore_blocked_error(
    workspace_name: &WorkspaceNameBuf,
    workspace_path: &std::path::Path,
) -> CommandError {
    user_error(format!(
        "Cannot restore workspace '{}' because directory '{}' already exists",
        workspace_name.as_symbol(),
        workspace_path.display()
    ))
    .hinted("Move the directory away, then retry `jj undo`.")
}

pub(crate) fn remove_workspace_directories(
    ui: &mut Ui,
    workspace_command: &WorkspaceCommandHelper,
    old_view: &View,
    new_view: &View,
) -> Result<(), CommandError> {
    let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;
    for workspace_name in old_view
        .wc_commit_ids
        .keys()
        .filter(|workspace_name| !new_view.wc_commit_ids.contains_key(*workspace_name))
        .cloned()
        .collect_vec()
    {
        let Some(rel_path) = workspace_store.get_workspace_path(&workspace_name)? else {
            continue;
        };
        let Ok(workspace_path) =
            normalize_maybe_missing(workspace_command.repo_path().join(rel_path))
        else {
            continue;
        };
        if workspace_path.join(".jj").join("repo").is_dir() || !workspace_path.exists() {
            continue;
        }
        if let Err(err) = std::fs::remove_dir_all(&workspace_path) {
            writeln!(
                ui.warning_default(),
                r#"Failed to remove workspace directory "{}": {err}"#,
                workspace_path.display()
            )?;
        } else {
            writeln!(
                ui.status(),
                r#"Removed workspace directory "{}"."#,
                workspace_path.display()
            )?;
        }
    }
    Ok(())
}

fn normalize_maybe_missing(path: std::path::PathBuf) -> Result<std::path::PathBuf, CommandError> {
    if path.exists() {
        return Ok(dunce::canonicalize(path)?);
    }
    let Some(parent) = path.parent() else {
        return Ok(path);
    };
    let Some(file_name) = path.file_name() else {
        return Ok(dunce::canonicalize(parent)?);
    };
    Ok(dunce::canonicalize(parent)?.join(file_name))
}
