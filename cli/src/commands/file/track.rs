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

use std::io;
use std::io::Write as _;

use indoc::writedoc;
use itertools::Itertools as _;
use jj_lib::matchers::Matcher;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::working_copy::SnapshotStats;
use jj_lib::working_copy::UntrackedReason;
use tracing::instrument;

use crate::cli_util::print_untracked_files;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Start tracking specified paths in the working copy
///
/// Without arguments, all paths that are not ignored will be tracked.
///
/// New files in the working copy can be automatically tracked.  
/// You can configure which paths to automatically track by setting
/// `snapshot.auto-track` (e.g. to `"none()"` or `"glob:**/*.rs"`). Files that
/// don't match the pattern can be manually tracked using this command. The
/// default pattern is `all()` and this command has no effect.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileTrackArgs {
    /// Paths to track
    #[arg(required = true, value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,
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
    let options = workspace_command.snapshot_options_with_start_tracking_matcher(&matcher)?;

    let mut tx = workspace_command.start_transaction().into_inner();
    let (mut locked_ws, _wc_commit) = workspace_command.start_working_copy_mutation()?;
    let (_tree_id, track_stats) = locked_ws.locked_wc().snapshot(&options)?;
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
        &matcher,
    )?;
    Ok(())
}

pub fn print_track_snapshot_stats(
    ui: &Ui,
    auto_stats: SnapshotStats,
    track_stats: SnapshotStats,
    path_converter: &RepoPathUiConverter,
    explicit_track_matcher: &dyn Matcher,
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

    print_untracked_files(
        ui,
        &merged_untracked_paths,
        path_converter,
        Some(explicit_track_matcher),
    )?;

    // Print hint about large files if needed
    let (large_files, sizes): (Vec<_>, Vec<_>) = merged_untracked_paths
        .iter()
        .filter_map(|(path, reason)| match reason {
            UntrackedReason::FileTooLarge { size, .. } => Some((path, *size)),
            UntrackedReason::FileNotAutoTracked | UntrackedReason::FileInTrackMatcherButIgnored => {
                None
            }
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

    // Print hint about ignored files if needed
    if merged_untracked_paths
        .iter()
        .filter(|(_, reason)| matches!(reason, UntrackedReason::FileInTrackMatcherButIgnored))
        .peekable()
        .peek()
        .is_some()
    {
        writedoc!(
            ui.hint_default(),
            "It appears at least one of the files you tried to track is matched by a pattern in \
             one of the .gitignore files in this repo; this is a current limitation of jj, please \
             try removing the pattern from your gitignore file temporarily then start tracking \
             that file.\n"
        )?;
    }
    Ok(())
}
