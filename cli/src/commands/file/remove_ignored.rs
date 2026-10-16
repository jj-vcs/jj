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

use std::collections::BTreeSet;
use std::io::Write as _;

use jj_lib::repo_path::RepoPathBuf;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Remove files ignored by .gitignore in the working copy
///
/// Without arguments, all ignored paths are removed. Pass one or more paths
/// to restrict the removal to a known-safe subset (e.g. a single
/// directory), leaving other ignored content (such as a build directory
/// you're still using) untouched. Use `jj file list-ignored` with the same
/// paths beforehand to preview what would be removed.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileRemoveIgnoredArgs {
    /// Only remove ignored paths matching these filesets
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_remove_ignored(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileRemoveIgnoredArgs,
) -> Result<(), CommandError> {
    let (workspace_command, snapshot_stats, _) = command.workspace_helper_with_stats(ui).await?;
    let matcher = workspace_command
        .parse_file_patterns(ui, &args.paths)?
        .to_matcher();
    let workspace_root = workspace_command.workspace_root();
    let mut removed_count = 0;
    let mut removed_parents: BTreeSet<RepoPathBuf> = BTreeSet::new();
    // `snapshot_stats.ignored_paths` is the exact same list shown by
    // `file list-ignored`: we remove exactly what that command lists (after
    // filtering by `args.paths`, if given), no more and no less. A directory
    // is only ever reported as a single entry there when it's fully covered
    // by a `.gitignore` directory pattern (in which case everything
    // underneath it is guaranteed ignored); an ordinary directory that
    // merely ends up containing only individually-ignored files (e.g. via a
    // `*.log` pattern) is reported file by file, since it could also contain
    // an unrelated, not-yet-ignored file that must not be touched.
    for ignored in &snapshot_stats.ignored_paths {
        let path_matches = if ignored.is_dir {
            // We're about to remove the whole directory as a unit, so it's
            // enough for the fileset to be relevant anywhere under it.
            !matcher.visit(&ignored.path).is_nothing()
        } else {
            matcher.matches(&ignored.path)
        };
        if !path_matches {
            continue;
        }
        let disk_path = ignored
            .path
            .to_fs_path(workspace_root)
            .map_err(|err| user_error(format!("Invalid path: {err}")))?;
        let Ok(metadata) = disk_path.symlink_metadata() else {
            // The file or directory was already removed (possibly
            // concurrently). Treat it as already deleted.
            continue;
        };
        if ignored.is_dir || metadata.is_dir() {
            std::fs::remove_dir_all(&disk_path).map_err(|err| {
                user_error(format!(
                    "Failed to remove directory {}: {err}",
                    disk_path.display()
                ))
            })?;
        } else {
            std::fs::remove_file(&disk_path).map_err(|err| {
                user_error(format!(
                    "Failed to remove file {}: {err}",
                    disk_path.display()
                ))
            })?;
        }
        removed_count += 1;
        if let Some(parent) = ignored.path.parent() {
            removed_parents.insert(parent.to_owned());
        }
    }
    // Clean up directories that became empty as a side effect of the
    // removals above (e.g. a directory that only ever contained
    // individually-ignored files). This never removes anything beyond what
    // was already deleted above: each directory is only removed once it's
    // verified, on disk, to be empty; we stop climbing as soon as we hit a
    // directory that still has content (tracked, untracked, or otherwise),
    // or the workspace root.
    for parent in removed_parents {
        let mut dir = Some(parent);
        while let Some(current) = dir {
            if current.is_root() {
                break;
            }
            let disk_dir = current
                .to_fs_path(workspace_root)
                .map_err(|err| user_error(format!("Invalid path: {err}")))?;
            match std::fs::remove_dir(&disk_dir) {
                Ok(()) => dir = current.parent().map(ToOwned::to_owned),
                Err(_) => break,
            }
        }
    }
    writeln!(
        ui.status(),
        "Removed {removed_count} ignored file(s) from the working copy."
    )?;
    Ok(())
}
