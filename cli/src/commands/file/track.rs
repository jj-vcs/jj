// Copyright 2024 The Jujutsu Authors
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
use std::fs::File;
use std::io;
use std::io::Write as _;
use std::path::Path;

use indoc::writedoc;
use itertools::Itertools as _;
use jj_lib::backend::CopyId;
use jj_lib::backend::TreeValue;
use jj_lib::file_util::BlockingAsyncReader;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTreeBuilder;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathComponent;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::store::Store;
use jj_lib::working_copy::SnapshotStats;
use jj_lib::working_copy::UntrackedReason;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::print_untracked_files;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Start tracking specified paths in the working copy
///
/// Without arguments, all paths that are not ignored will be tracked.
///
/// By default, new files in the working copy are automatically tracked, so
/// this command has no effect.
/// You can configure which paths to automatically track by setting
/// `snapshot.auto-track` (e.g. to `"none()"` or `"glob:**/*.rs"`). Files that
/// don't match the pattern can be manually tracked using this command. The
/// default pattern is `all()`.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileTrackArgs {
    /// Paths to track
    #[arg(required = true, value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,

    /// Track paths even if they're ignored or too large
    ///
    /// By default, `jj file track` will not track files that are ignored by
    /// .gitignore or exceed the maximum file size. This flag overrides those
    /// restrictions, explicitly tracking the specified files.
    #[arg(long)]
    include_ignored: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_file_track(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileTrackArgs,
) -> Result<(), CommandError> {
    let (mut workspace_command, auto_stats) = command.workspace_helper_with_stats(ui)?;
    let matcher = workspace_command
        .parse_file_patterns(ui, &args.paths)?
        .to_matcher();

    if args.include_ignored {
        // Bypass ignore rules by manually building the tree from disk
        let store = workspace_command.repo().store().clone();
        let working_copy_path = workspace_command.workspace_root().to_path_buf();
        let auto_tracking_matcher = workspace_command.auto_tracking_matcher(ui)?;
        let options = workspace_command
            .snapshot_options_with_start_tracking_matcher(&auto_tracking_matcher)?;

        let mut tx = workspace_command.start_transaction().into_inner();
        let (mut locked_ws, wc_commit) = workspace_command.start_working_copy_mutation()?;

        let mut paths = Vec::new();
        walk_dir_recursive(
            matcher.as_ref(),
            &working_copy_path,
            RepoPath::root(),
            &mut paths,
        )?;

        let mut tree_builder = MergedTreeBuilder::new(wc_commit.tree_id().clone());

        for path in paths {
            let disk_path = path.to_fs_path(&working_copy_path).map_err(|err| {
                user_error(format!("Failed to convert path to filesystem path: {err}"))
            })?;
            let tree_value = add_file_to_tree(&store, &path, &disk_path).block_on()?;
            tree_builder.set_or_remove(path, Merge::normal(tree_value));
        }

        let new_tree_id = tree_builder.write_tree(&store)?;
        let new_commit = tx
            .repo_mut()
            .rewrite_commit(&wc_commit)
            .set_tree_id(new_tree_id)
            .write()?;

        // Reset working copy state to new tree (doesn't touch files on disk)
        locked_ws.locked_wc().reset(&new_commit).block_on()?;

        // Snapshot to capture any concurrent changes
        let (_wc_tree_id, track_stats) = locked_ws.locked_wc().snapshot(&options).block_on()?;

        let num_rebased = tx.repo_mut().rebase_descendants()?;
        if num_rebased > 0 {
            writeln!(ui.status(), "Rebased {num_rebased} descendant commits")?;
        }
        let repo = tx.commit("track paths")?;
        locked_ws.finish(repo.op_id().clone())?;
        print_track_snapshot_stats(
            ui,
            auto_stats,
            track_stats,
            workspace_command.env().path_converter(),
        )?;
    } else {
        // Let snapshot track files normally (respecting ignore rules)
        let options = workspace_command.snapshot_options_with_start_tracking_matcher(&matcher)?;

        let mut tx = workspace_command.start_transaction().into_inner();
        let (mut locked_ws, _wc_commit) = workspace_command.start_working_copy_mutation()?;
        let (_tree_id, track_stats) = locked_ws.locked_wc().snapshot(&options).block_on()?;
        let num_rebased = tx.repo_mut().rebase_descendants()?;
        if num_rebased > 0 {
            writeln!(ui.status(), "Rebased {num_rebased} descendant commits")?;
        }
        let repo = tx.commit("track paths")?;
        locked_ws.finish(repo.op_id().clone())?;
        print_track_snapshot_stats(
            ui,
            auto_stats,
            track_stats,
            workspace_command.env().path_converter(),
        )?;
    }
    Ok(())
}

/// Recursively walk a directory and collect matching file paths
///
/// This is a simplified version of FileSnapshotter::visit_directory that
/// intentionally bypasses ignore checking.
fn walk_dir_recursive(
    matcher: &dyn Matcher,
    working_copy_path: &Path,
    repo_dir: &RepoPath,
    paths: &mut Vec<RepoPathBuf>,
) -> Result<(), CommandError> {
    let disk_dir = repo_dir
        .to_fs_path(working_copy_path)
        .map_err(|err| user_error(format!("Failed to convert path to filesystem path: {err}")))?;

    // Read directory entries
    let entries = fs::read_dir(&disk_dir).map_err(|err| {
        user_error(format!(
            "Failed to read directory {}: {}",
            disk_dir.display(),
            err
        ))
    })?;

    for entry in entries {
        let entry = entry.map_err(|err| {
            user_error(format!(
                "Failed to read directory entry in {}: {}",
                disk_dir.display(),
                err
            ))
        })?;

        let file_name = entry.file_name();
        let name_string = file_name.to_str().ok_or_else(|| {
            user_error(format!(
                "Invalid UTF-8 in filename: {}",
                file_name.to_string_lossy()
            ))
        })?;

        // Skip special directories
        if name_string == ".jj" || name_string == ".git" {
            continue;
        }

        let name = RepoPathComponent::new(name_string)
            .map_err(|_| user_error(format!("Invalid path component: {name_string}")))?;
        let repo_path = repo_dir.join(name);
        let disk_path = entry.path();

        let file_type = entry.file_type().map_err(|err| {
            user_error(format!(
                "Failed to get file type for {}: {}",
                disk_path.display(),
                err
            ))
        })?;

        if file_type.is_dir() {
            // Check if we should visit this directory
            if !matcher.visit(&repo_path).is_nothing() {
                walk_dir_recursive(matcher, working_copy_path, &repo_path, paths)?;
            }
        } else if matcher.matches(&repo_path) {
            // It's a file (or symlink) and it matches
            paths.push(repo_path);
        }
    }

    Ok(())
}

/// Read a file from disk, write it to the store, and return a TreeValue
async fn add_file_to_tree(
    store: &Store,
    repo_path: &RepoPath,
    disk_path: &Path,
) -> Result<TreeValue, CommandError> {
    let metadata = disk_path
        .symlink_metadata()
        .map_err(|err| user_error(format!("Failed to read {}: {}", disk_path.display(), err)))?;

    if metadata.is_symlink() {
        let target = disk_path.read_link().map_err(|err| {
            user_error(format!(
                "Failed to read symlink {}: {}",
                disk_path.display(),
                err
            ))
        })?;
        let target_str = target.to_str().ok_or_else(|| {
            user_error(format!(
                "Symlink target is not valid UTF-8: {}",
                target.to_string_lossy()
            ))
        })?;
        let id = store
            .write_symlink(repo_path, target_str)
            .await
            .map_err(|err| user_error(format!("Failed to write symlink to store: {err}")))?;
        Ok(TreeValue::Symlink(id))
    } else if metadata.is_file() {
        let file = File::open(disk_path).map_err(|err| {
            user_error(format!(
                "Failed to open file {}: {}",
                disk_path.display(),
                err
            ))
        })?;
        let mut reader = BlockingAsyncReader::new(file);
        let id = store
            .write_file(repo_path, &mut reader)
            .await
            .map_err(|err| user_error(format!("Failed to write file to store: {err}")))?;

        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;

        Ok(TreeValue::File {
            id,
            executable,
            copy_id: CopyId::placeholder(),
        })
    } else {
        Err(user_error(format!(
            "{} is not a regular file or symlink",
            disk_path.display()
        )))
    }
}

pub fn print_track_snapshot_stats(
    ui: &Ui,
    auto_stats: SnapshotStats,
    track_stats: SnapshotStats,
    path_converter: &RepoPathUiConverter,
) -> io::Result<()> {
    let mut merged_untracked_paths = auto_stats.untracked_paths;
    for (path, reason) in track_stats
        .untracked_paths
        .into_iter()
        // focus on files that are now tracked with `file track`
        .filter(|(_, reason)| !matches!(reason, UntrackedReason::FileNotAutoTracked))
    {
        // if the path was previously rejected because it wasn't tracked, update its
        // reason
        merged_untracked_paths.insert(path, reason);
    }

    print_untracked_files(ui, &merged_untracked_paths, path_converter)?;

    let (large_files, sizes): (Vec<_>, Vec<_>) = merged_untracked_paths
        .iter()
        .filter_map(|(path, reason)| match reason {
            UntrackedReason::FileTooLarge { size, .. } => Some((path, *size)),
            UntrackedReason::FileNotAutoTracked => None,
        })
        .unzip();
    if let Some(size) = sizes.iter().max() {
        let large_files_list = large_files
            .iter()
            .map(|path| path_converter.format_file_path(path))
            .join(" ");
        writedoc!(
            ui.hint_default(),
            r"
            This is to prevent large files from being added by accident. You can fix this by:
              - Adding the file to `.gitignore`
              - Run `jj config set --repo snapshot.max-new-file-size {size}`
                This will increase the maximum file size allowed for new files, in this repository only.
              - Run `jj --config snapshot.max-new-file-size={size} file track {large_files_list}`
                This will increase the maximum file size allowed for new files, for this command only.
            "
        )?;
    }
    Ok(())
}
