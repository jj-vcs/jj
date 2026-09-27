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

#[cfg(feature = "git")]
use std::io::Write as _;
use std::slice;
use std::time::Duration;
use std::time::SystemTime;

#[cfg(feature = "git")]
use jj_lib::git_backend::GitGcOutcome;
use jj_lib::repo::Repo as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Run backend-dependent maintenance.
///
/// Run `jj op abandon ..<some old operation>` before this command to make old
/// operations eligible for removal. The Git backend only publishes a bounded
/// additive pack; it does not delete Git objects because jj cannot exclude
/// independent Git readers and writers. Git object reclamation is deferred.
#[derive(clap::Args, Clone, Debug)]
pub struct UtilGcArgs {
    /// Time threshold
    ///
    /// By default, only obsolete operations older than 2 weeks are pruned.
    /// Git objects newer than this threshold are retained during repacking.
    ///
    /// Only the string "now" can be passed to this parameter. Support for
    /// arbitrary absolute and relative timestamps will come in a subsequent
    /// release.
    #[arg(long)]
    expire: Option<String>,
}

pub async fn cmd_util_gc(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilGcArgs,
) -> Result<(), CommandError> {
    if !command.is_at_head_operation() {
        return Err(user_error(
            "Cannot garbage collect from a non-head operation",
        ));
    }
    let keep_newer = match args.expire.as_deref() {
        None => SystemTime::now() - Duration::from_secs(14 * 86400),
        Some("now") => SystemTime::now() - Duration::ZERO,
        _ => return Err(user_error("--expire only accepts 'now'")),
    };
    let workspace_command = command.workspace_helper(ui).await?;

    let repo = workspace_command.repo();
    repo.op_store()
        .gc(slice::from_ref(repo.op_id()), keep_newer)
        .await?;
    #[cfg(feature = "git")]
    if let Ok(git_backend) = jj_lib::git::get_git_backend(repo.store()) {
        match git_backend.gc(repo.index(), keep_newer)? {
            GitGcOutcome::Repacked {
                pack_bytes,
                index_bytes,
            } => writeln!(
                ui.status(),
                "Git objects repacked ({pack_bytes} pack bytes, {index_bytes} index bytes); \
                 reclamation deferred."
            )?,
            GitGcOutcome::Deferred { reason } => {
                writeln!(ui.status(), "Git object reclamation deferred: {reason}.")?;
            }
        }
    }
    Ok(())
}
