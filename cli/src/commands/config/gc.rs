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

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use jj_lib::secure_config;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Find and optionally delete repo-level config directories whose repo path
/// no longer exists.
// TODO: also gc workspace-level config directories under
// `$HOME/.config/jj/workspaces/`.
// TODO: support a path argument to filter which repo paths to delete, e.g.
// `jj config gc /tmp` to only delete repo configs whose recorded path is
// under `/tmp`.
#[derive(clap::Args, Clone, Debug)]
pub struct ConfigGcArgs {}

#[instrument(skip_all)]
pub async fn cmd_config_gc(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &ConfigGcArgs,
) -> Result<(), CommandError> {
    let root = command
        .config_env()
        .repo_configs_root_dir()
        .ok_or_else(|| user_error("No config directory found"))?;

    let missing = find_missing_repo_configs(&root)?;

    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "Missing repo configs (repo path no longer exists):"
        )?;
        if missing.is_empty() {
            writeln!(formatter, "  (none)")?;
        } else {
            for (config_dir, repo_path) in &missing {
                writeln!(formatter, "  {}", config_dir.display())?;
                writeln!(formatter, "    repo path: {}", repo_path.display())?;
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    let prompt = format!("Delete {} missing repo config directories?", missing.len());
    if !ui.prompt_yes_no(&prompt, Some(false))? {
        writeln!(ui.status(), "Aborted; nothing was deleted.")?;
        return Ok(());
    }

    let mut deleted = 0;
    for (config_dir, _) in &missing {
        match secure_config::remove_repo_config_dir(config_dir) {
            Ok(()) => deleted += 1,
            Err(err) => writeln!(
                ui.warning_default(),
                "Failed to delete {}: {err}",
                config_dir.display()
            )?,
        }
    }
    writeln!(ui.status(), "Deleted {deleted} config directories.")?;
    Ok(())
}

/// Returns `(config_dir, repo_path)` pairs for every per-repo config
/// directory under `root` whose recorded repo path no longer exists on
/// disk. The list is sorted by config directory name.
pub(crate) fn find_missing_repo_configs(
    root: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, CommandError> {
    if !root.try_exists().map_err(|err| {
        user_error_with_message(format!("Failed to check {}", root.display()), err)
    })? {
        return Ok(Vec::new());
    }
    let read_dir = fs::read_dir(root).map_err(|err| {
        user_error_with_message(format!("Failed to read {}", root.display()), err)
    })?;

    let mut missing = Vec::new();
    for dir_entry in read_dir {
        let dir_entry = dir_entry.map_err(|err| {
            user_error_with_message(format!("Failed to read {}", root.display()), err)
        })?;
        let config_dir = dir_entry.path();
        if !config_dir.is_dir() {
            continue;
        }
        let Ok(metadata) = secure_config::read_metadata(&config_dir) else {
            continue;
        };
        let Ok(Some(repo_path)) = secure_config::metadata_path(&metadata) else {
            continue;
        };
        // Treat "exists" errors (e.g. permission denied) as "still exists" so
        // we don't propose deleting configs we can't actually verify.
        if !repo_path.try_exists().unwrap_or(true) {
            missing.push((config_dir, repo_path.to_path_buf()));
        }
    }
    missing.sort();
    Ok(missing)
}
