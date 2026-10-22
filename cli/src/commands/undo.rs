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
use std::pin::pin;
use std::slice;

use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::OperationId;
use jj_lib::op_walk;
use jj_lib::operation::Operation;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::ReadonlyRepo;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
#[cfg(feature = "git")]
use crate::commands::git::is_push_operation;
use crate::commands::operation::DEFAULT_REVERT_WHAT;
use crate::commands::operation::revert_operation;
use crate::ui::Ui;

/// Undo the last operation in the current workspace
///
/// This reverts the most recent operation performed in the current workspace,
/// leaving changes made by operations in other workspaces in place. If `jj
/// undo` is used repeatedly, it will revert increasingly older operations,
/// going further back into the past.
///
/// There is also a complementary `jj redo` command that would instead move in
/// the direction of the future after one or more `jj undo`s.
///
/// Use `jj op log` to visualize the log of past operations, including a
/// detailed description of any past undo/redo operations. See also `jj op
/// revert` to revert a specific operation by its id, and `jj op restore` to
/// restore the whole repo to an older operation.
#[derive(clap::Args, Clone, Debug)]
pub struct UndoArgs {
    /// Deprecated; has no effect
    #[arg(long, hide = true)]
    allow_cross_workspace: bool,
}

pub(crate) const UNDO_OP_DESC_PREFIX: &str = "undo: revert operation ";
pub(crate) const REDO_OP_DESC_PREFIX: &str = "redo: revert operation ";
// Older versions of jj restored the repo to an earlier operation instead of
// reverting an operation.
const LEGACY_UNDO_OP_DESC_PREFIX: &str = "undo: restore to operation ";
const LEGACY_REDO_OP_DESC_PREFIX: &str = "redo: restore to operation ";

/// How an operation affects the undo- and redo-stacks.
enum StackEntry {
    /// An operation not created by `jj undo` or `jj redo`
    Regular,
    /// An operation created by `jj undo`, which reverted the given operation
    Undo(OperationId),
    /// An operation created by `jj redo`, which reverted the given
    /// undo-operation
    Redo(OperationId),
    /// An operation created by `jj undo` or `jj redo` in older versions of jj,
    /// which restored the repo to the given operation
    LegacyRestore {
        restored: OperationId,
        is_undo: bool,
    },
}

impl StackEntry {
    /// Parses the operation's description. A description with one of the
    /// undo/redo prefixes but without a valid operation ID after it (e.g. one
    /// written by another tool) is treated as a regular operation.
    fn parse(op: &Operation) -> Self {
        let description = &op.metadata().description;
        let parse_id = |prefix: &str| {
            description
                .strip_prefix(prefix)
                .and_then(OperationId::try_from_hex)
        };
        if let Some(id) = parse_id(UNDO_OP_DESC_PREFIX) {
            Self::Undo(id)
        } else if let Some(id) = parse_id(REDO_OP_DESC_PREFIX) {
            Self::Redo(id)
        } else if let Some(restored) = parse_id(LEGACY_UNDO_OP_DESC_PREFIX) {
            Self::LegacyRestore {
                restored,
                is_undo: true,
            }
        } else if let Some(restored) = parse_id(LEGACY_REDO_OP_DESC_PREFIX) {
            Self::LegacyRestore {
                restored,
                is_undo: false,
            }
        } else {
            Self::Regular
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StackKind {
    Undo,
    Redo,
}

/// Finds the operation `jj undo` or `jj redo` should revert in the given
/// workspace.
///
/// For `jj undo`, that's the most recent regular operation performed in the
/// workspace that hasn't been undone. For `jj redo`, it's the most recent
/// undo-operation performed in the workspace that hasn't been redone, as long
/// as no regular operation was performed in the workspace after it.
///
/// Operations performed in other workspaces are skipped, as are merge
/// operations. Undo- and redo-operations cancel the operation they reverted,
/// so repeated `jj undo` walks further back into the past. This also means
/// that `jj undo` "jumps over" old undo-stacks. Consider this op log, where
/// all operations were performed in the same workspace:
///
/// ```text
/// G "undo: revert operation B"
/// F "undo: revert operation E"
/// E
/// D "undo: revert operation C"
/// C
/// B
/// A
/// ```
///
/// When G was created, F had canceled E and D had canceled C, so the most
/// recent operation that hadn't been undone was B.
pub(crate) async fn find_stack_target(
    repo: &ReadonlyRepo,
    workspace_name: &WorkspaceName,
    kind: StackKind,
) -> Result<Option<Operation>, CommandError> {
    let mut canceled: HashSet<OperationId> = HashSet::new();
    let mut ops = pin!(op_walk::walk_ancestors(slice::from_ref(repo.operation())));
    while let Some(op) = ops.try_next().await? {
        if canceled.contains(op.id()) {
            continue;
        }
        let is_own = op.metadata().workspace_name.as_deref() == Some(workspace_name);
        match StackEntry::parse(&op) {
            StackEntry::Regular => {
                if !is_own || op.parent_ids().len() != 1 {
                    continue;
                }
                return Ok(match kind {
                    StackKind::Undo => Some(op),
                    StackKind::Redo => None,
                });
            }
            StackEntry::Undo(reverted) => {
                if !is_own {
                    continue;
                }
                match kind {
                    StackKind::Undo => {
                        canceled.insert(reverted);
                    }
                    StackKind::Redo => return Ok(Some(op)),
                }
            }
            StackEntry::Redo(reverted) => {
                if is_own {
                    canceled.insert(reverted);
                }
            }
            StackEntry::LegacyRestore { restored, is_undo } => {
                if is_own && is_undo && kind == StackKind::Redo {
                    return Ok(Some(op));
                }
                // Everything since the restored operation was discarded,
                // regardless of which workspace it was performed in.
                let restored = repo.loader().load_operation(&restored).await?;
                let discarded: Vec<_> =
                    op_walk::walk_ancestors_range(&op.parents().await?, slice::from_ref(&restored))
                        .map_ok(|op| op.id().clone())
                        .try_collect()
                        .await?;
                canceled.extend(discarded);
            }
        }
    }
    Ok(None)
}

pub async fn cmd_undo(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UndoArgs,
) -> Result<(), CommandError> {
    if args.allow_cross_workspace {
        writeln!(
            ui.warning_default(),
            "`--allow-cross-workspace` is deprecated and has no effect; `jj undo` only considers \
             operations performed in the current workspace"
        )?;
    }
    let mut workspace_command = command.workspace_helper(ui).await?;
    let target_op = find_stack_target(
        workspace_command.repo(),
        workspace_command.workspace_name(),
        StackKind::Undo,
    )
    .await?
    .ok_or_else(|| user_error("Nothing to undo in this workspace"))?;

    #[cfg(feature = "git")]
    if is_push_operation(&target_op) {
        writeln!(
            ui.warning_default(),
            "Undoing a push operation often leads to conflicted bookmarks."
        )?;
        writeln!(ui.hint_default(), "To avoid this, run `jj redo` now.")?;
    }
    if target_op
        .metadata()
        .description
        .starts_with("remove workspace")
    {
        writeln!(
            ui.warning_default(),
            "Undoing a workspace removal restores the workspace, but does not restore the \
             workspace directory."
        )?;
        writeln!(
            ui.hint_default(),
            "Run `jj workspace forget` then `jj workspace add` to recreate the directory."
        )?;
    }

    let target_op_parent = target_op
        .parents()
        .await?
        .into_iter()
        .exactly_one()
        .map_err(|_| internal_error("Operation to undo should have a single parent"))?;

    let mut tx = workspace_command.start_transaction();
    revert_operation(&mut tx, &target_op, &target_op_parent, &DEFAULT_REVERT_WHAT).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        let template = tx.base_workspace_helper().operation_summary_template();
        write!(formatter, "Undid operation: ")?;
        template.format(&target_op, formatter.as_mut())?;
        writeln!(formatter)?;
    }
    tx.finish(ui, format!("{UNDO_OP_DESC_PREFIX}{}", target_op.id().hex()))
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pollster::FutureExt as _;
    use testutils::TestRepo;
    use testutils::TestResult;

    use super::*;

    /// Commits an operation in the default workspace that doesn't change the
    /// repo. `find_stack_target()` only looks at the descriptions, workspace
    /// names, and the operation graph.
    fn write_op(repo: &Arc<ReadonlyRepo>, description: &str) -> TestResult<Arc<ReadonlyRepo>> {
        write_op_in_workspace(repo, WorkspaceName::DEFAULT, description)
    }

    fn write_op_in_workspace(
        repo: &Arc<ReadonlyRepo>,
        workspace_name: &WorkspaceName,
        description: &str,
    ) -> TestResult<Arc<ReadonlyRepo>> {
        let mut tx = repo.start_transaction();
        tx.set_workspace_name(workspace_name);
        Ok(tx.commit(description).block_on()?)
    }

    /// Commits an undo-operation like older versions of jj did, i.e. one that
    /// restored the repo to `restored`.
    fn write_legacy_undo_op(
        repo: &Arc<ReadonlyRepo>,
        restored: &ReadonlyRepo,
    ) -> TestResult<Arc<ReadonlyRepo>> {
        let description = format!("{LEGACY_UNDO_OP_DESC_PREFIX}{}", restored.op_id().hex());
        write_op(repo, &description)
    }

    /// Commits a redo-operation like older versions of jj did, i.e. one that
    /// restored the repo to `restored`.
    fn write_legacy_redo_op(
        repo: &Arc<ReadonlyRepo>,
        restored: &ReadonlyRepo,
    ) -> TestResult<Arc<ReadonlyRepo>> {
        let description = format!("{LEGACY_REDO_OP_DESC_PREFIX}{}", restored.op_id().hex());
        write_op(repo, &description)
    }

    fn find_target(repo: &ReadonlyRepo, kind: StackKind) -> TestResult<Option<OperationId>> {
        let op = find_stack_target(repo, WorkspaceName::DEFAULT, kind)
            .block_on()
            .map_err(|err| err.error)?;
        Ok(op.map(|op| op.id().clone()))
    }

    #[test]
    fn test_legacy_undo() -> TestResult {
        let test_repo = TestRepo::init();
        let repo_a = write_op(&test_repo.repo, "a")?;
        let repo_b = write_op(&repo_a, "b")?;
        let repo_c = write_op(&repo_b, "c")?;
        // Undo C by restoring B
        let repo_undo = write_legacy_undo_op(&repo_c, &repo_b)?;

        // C was discarded, so B is the next operation to undo
        assert_eq!(
            find_target(&repo_undo, StackKind::Undo)?,
            Some(repo_b.op_id().clone())
        );
        // The legacy undo-operation can be redone
        assert_eq!(
            find_target(&repo_undo, StackKind::Redo)?,
            Some(repo_undo.op_id().clone())
        );
        Ok(())
    }

    #[test]
    fn test_legacy_redo() -> TestResult {
        let test_repo = TestRepo::init();
        let repo_a = write_op(&test_repo.repo, "a")?;
        let repo_b = write_op(&repo_a, "b")?;
        let repo_c = write_op(&repo_b, "c")?;
        // Undo C by restoring B, then redo it by restoring C
        let repo_undo = write_legacy_undo_op(&repo_c, &repo_b)?;
        let repo_redo = write_legacy_redo_op(&repo_undo, &repo_c)?;

        // The undo-operation was discarded, so C can be undone again
        assert_eq!(
            find_target(&repo_redo, StackKind::Undo)?,
            Some(repo_c.op_id().clone())
        );
        assert_eq!(find_target(&repo_redo, StackKind::Redo)?, None);
        Ok(())
    }

    #[test]
    fn test_legacy_undo_jump_over_old_undo_stack() -> TestResult {
        let test_repo = TestRepo::init();
        let repo_a = write_op(&test_repo.repo, "a")?;
        let repo_b = write_op(&repo_a, "b")?;
        let repo_c = write_op(&repo_b, "c")?;
        // Undo C by restoring B
        let repo_undo1 = write_legacy_undo_op(&repo_c, &repo_b)?;
        let repo_e = write_op(&repo_undo1, "e")?;
        // Undo E by restoring B (older versions jumped over the undo-operation)
        let repo_undo2 = write_legacy_undo_op(&repo_e, &repo_b)?;
        assert_eq!(
            find_target(&repo_undo2, StackKind::Undo)?,
            Some(repo_b.op_id().clone())
        );
        // Undo B by restoring A
        let repo_undo3 = write_legacy_undo_op(&repo_undo2, &repo_a)?;
        assert_eq!(
            find_target(&repo_undo3, StackKind::Undo)?,
            Some(repo_a.op_id().clone())
        );
        assert_eq!(
            find_target(&repo_undo3, StackKind::Redo)?,
            Some(repo_undo3.op_id().clone())
        );
        Ok(())
    }

    #[test]
    fn test_redo_legacy_undo() -> TestResult {
        let test_repo = TestRepo::init();
        let repo_a = write_op(&test_repo.repo, "a")?;
        let repo_b = write_op(&repo_a, "b")?;
        let repo_c = write_op(&repo_b, "c")?;
        // Undo C by restoring B, then redo it with the current format
        let repo_undo = write_legacy_undo_op(&repo_c, &repo_b)?;
        let description = format!("{REDO_OP_DESC_PREFIX}{}", repo_undo.op_id().hex());
        let repo_redo = write_op(&repo_undo, &description)?;

        // The redo-operation canceled the legacy undo-operation, so C can be
        // undone again
        assert_eq!(
            find_target(&repo_redo, StackKind::Undo)?,
            Some(repo_c.op_id().clone())
        );
        assert_eq!(find_target(&repo_redo, StackKind::Redo)?, None);
        Ok(())
    }

    #[test]
    fn test_malformed_undo_description() -> TestResult {
        let test_repo = TestRepo::init();
        let repo_a = write_op(&test_repo.repo, "a")?;
        let description = format!("{UNDO_OP_DESC_PREFIX}not-an-id");
        let repo_b = write_op(&repo_a, &description)?;

        // The operation is treated as a regular operation
        assert_eq!(
            find_target(&repo_b, StackKind::Undo)?,
            Some(repo_b.op_id().clone())
        );
        assert_eq!(find_target(&repo_b, StackKind::Redo)?, None);
        Ok(())
    }

    #[test]
    fn test_legacy_undo_in_other_workspace() -> TestResult {
        let test_repo = TestRepo::init();
        let repo_a = write_op(&test_repo.repo, "a")?;
        let repo_b = write_op(&repo_a, "b")?;
        let repo_c = write_op(&repo_b, "c")?;
        // Undo C from another workspace by restoring B
        let description = format!("{LEGACY_UNDO_OP_DESC_PREFIX}{}", repo_b.op_id().hex());
        let repo_undo = write_op_in_workspace(&repo_c, "other".as_ref(), &description)?;

        // C was discarded even though the undo-operation was performed in
        // another workspace
        assert_eq!(
            find_target(&repo_undo, StackKind::Undo)?,
            Some(repo_b.op_id().clone())
        );
        // The undo-operation can only be redone from the other workspace
        assert_eq!(find_target(&repo_undo, StackKind::Redo)?, None);
        Ok(())
    }
}
