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

use std::io::Write as _;
use std::slice;
use std::time::Duration;
use std::time::SystemTime;

use jj_lib::gc;
use jj_lib::op_walk;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Run garbage collection on the repository.
///
/// Abandons old operations beyond the configured limits, garbage collects
/// unreachable objects, and rebuilds the commit index.
#[derive(clap::Args, Clone, Debug)]
pub struct GcArgs {}

pub async fn cmd_gc(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &GcArgs,
) -> Result<(), CommandError> {
    // GC reparents operations and updates op heads, which requires operating
    // at the current head. Running at a non-head operation with --at-op would
    // leave the working copy pointing to a stale operation that may no longer
    // exist after reparenting.
    if !command.is_at_head_operation() {
        return Err(user_error(
            "Cannot garbage collect from a non-head operation",
        ));
    }
    let workspace_command = command.workspace_helper(ui).await?;
    run_gc(ui, workspace_command.repo(), workspace_command.repo_path()).await
}

pub(crate) async fn run_gc(
    ui: &mut Ui,
    repo: &jj_lib::repo::ReadonlyRepo,
    repo_path: &std::path::Path,
) -> Result<(), CommandError> {
    let settings = gc::GcSettings {
        frequency_days: 0,
        operation_min_count: repo.settings().get_int("gc.min-operation-count")?,
        operation_expiry_days: repo.settings().get_int("gc.operation-expiry-days")?,
    };

    writeln!(ui.status(), "Running garbage collection...")?;

    let op_store = repo.op_store();
    let op_heads_store = repo.op_heads_store();

    let head_ops = op_walk::get_current_head_ops(op_store, op_heads_store.as_ref()).await?;
    let expiry_cutoff = settings.operation_expiry_cutoff();

    let discard_op =
        gc::find_discard_op(&head_ops, settings.operation_min_count, expiry_cutoff).await?;

    if let Some(discard_op) = discard_op {
        let root_op = repo.loader().root_operation().await;

        let current_head_ops = head_ops;
        let stats = op_walk::reparent_range(
            op_store.as_ref(),
            &[discard_op],
            &current_head_ops,
            &root_op,
        )
        .await?;

        for (old, new_id) in std::iter::zip(&current_head_ops, &stats.new_head_ids) {
            if old.id() != new_id {
                op_heads_store
                    .update_op_heads(slice::from_ref(old.id()), new_id)
                    .await
                    .map_err(|err| user_error(err.to_string()))?;
            }
        }
        writeln!(
            ui.status(),
            "Abandoned {} operations, reparented {} operations.",
            stats.unreachable_count,
            stats.rewritten_count,
        )?;
    }

    let keep_newer = SystemTime::now() - Duration::ZERO;
    expire_unreachable(repo, keep_newer).await?;
    reindex(repo).await?;
    gc::write_last_gc_time(repo_path).map_err(|err| user_error(err.to_string()))?;

    writeln!(ui.status(), "Garbage collection complete.")?;
    Ok(())
}

/// Prunes unreachable operations and objects older than `keep_newer`.
pub(crate) async fn expire_unreachable(
    repo: &jj_lib::repo::ReadonlyRepo,
    keep_newer: std::time::SystemTime,
) -> Result<(), CommandError> {
    repo.op_store()
        .gc(slice::from_ref(repo.op_id()), keep_newer)
        .await
        .map_err(|err| user_error(err.to_string()))?;
    repo.store()
        .gc(repo.index(), keep_newer)
        .map_err(|err| user_error(err.to_string()))?;
    Ok(())
}

/// Rebuilds the commit index at the current operation.
pub(crate) async fn reindex(repo: &jj_lib::repo::ReadonlyRepo) -> Result<(), CommandError> {
    crate::commands::debug::reindex::reindex_at_operation(repo.loader(), repo.operation()).await?;
    Ok(())
}
