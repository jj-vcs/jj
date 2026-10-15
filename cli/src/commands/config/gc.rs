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

use itertools::Itertools as _;
use jj_lib::secure_config;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Find and optionally delete repo and workspace config directories whose paths
/// no longer exist.
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
    let repo_configs_root = command
        .config_env()
        .repo_configs_root_dir()
        .ok_or_else(|| user_error("No repo configs directory found"))?;
    let missing_repo_paths = find_configs_with_missing_paths(&repo_configs_root)?;

    let workspace_configs_root = command
        .config_env()
        .workspace_configs_root_dir()
        .ok_or_else(|| user_error("No workspace configs directory found"))?;
    let missing_workspace_paths = find_configs_with_missing_paths(&workspace_configs_root)?;

    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(
            formatter,
            "Missing repo configs (repo path no longer exists):"
        )?;
        if missing_repo_paths.is_empty() {
            writeln!(formatter, "  (none)")?;
        } else {
            for (config_dir, repo_path) in &missing_repo_paths {
                writeln!(formatter, "  {}", config_dir.display())?;
                writeln!(formatter, "    repo path: {}", repo_path.display())?;
            }
        }

        writeln!(
            formatter,
            "Missing workspace configs (workspace path no longer exists):"
        )?;
        if missing_workspace_paths.is_empty() {
            writeln!(formatter, "  (none)")?;
        } else {
            for (config_dir, workspace_path) in &missing_workspace_paths {
                writeln!(formatter, "  {}", config_dir.display())?;
                writeln!(
                    formatter,
                    "    workspace path: {}",
                    workspace_path.display()
                )?;
            }
        }
    }

    let missing = missing_repo_paths
        .into_iter()
        .chain(missing_workspace_paths)
        .map(|(config_dir, _)| config_dir)
        .collect_vec();
    if missing.is_empty() {
        return Ok(());
    }

    if !ui.prompt_yes_no(
        &format!("Delete {} missing config directories?", missing.len()),
        Some(false),
    )? {
        writeln!(ui.status(), "Aborted; nothing was deleted.")?;
        return Ok(());
    }

    let mut deleted = 0;
    for config_dir in &missing {
        match secure_config::remove_config_dir(config_dir) {
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

/// Returns `(config_dir, path)` pairs for every config directory under `root`
/// whose recorded repo or workspace path no longer exists on disk. The list is
/// sorted by config directory name.
pub(crate) fn find_configs_with_missing_paths(
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
        let Ok(Some(path)) = secure_config::metadata_path(&metadata) else {
            continue;
        };
        // Treat "exists" errors (e.g. permission denied) as "still exists" so
        // we don't propose deleting configs we can't actually verify.
        if !path.try_exists().unwrap_or(true) {
            missing.push((config_dir, path.to_path_buf()));
        }
    }
    missing.sort();
    Ok(missing)
}
