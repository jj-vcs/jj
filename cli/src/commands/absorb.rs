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

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use indoc::formatdoc;
use jj_lib::absorb::AbsorbSource;
use jj_lib::absorb::absorb_hunks;
use jj_lib::absorb::split_hunks_to_trees;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merge::Diff;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::diff_util::DiffFormat;
use crate::ui::Ui;

/// Move changes from a revision into the stack of mutable revisions
///
/// This command splits changes in the source revision and moves each change to
/// the closest mutable ancestor where the corresponding lines were modified
/// last. If the destination revision cannot be determined unambiguously, the
/// change will be left in the source revision.
///
/// With the `--interactive` option, only the selected changes will be
/// considered for absorption. This allows picking specific hunks to absorb
/// (which may then be distributed across multiple ancestors). The
/// `--tool` option can be used to select a different diff editor.
///
/// The source revision will be abandoned if all changes are absorbed into the
/// destination revisions, and if the source revision has no description.
///
/// The modification made by `jj absorb` can be reviewed by `jj op show -p`.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct AbsorbArgs {
    /// Source revision to absorb from
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    from: RevisionArg,

    /// Destination revisions to absorb into
    ///
    /// Only ancestors of the source revision will be considered.
    #[arg(
        long,
        short = 't',
        visible_alias = "to",
        default_value = "mutable()",
        value_name = "REVSETS"
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    into: Vec<RevisionArg>,

    /// Move only changes to these paths (instead of all paths)
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::modified_from_files))]
    paths: Vec<String>,

    /// Interactively choose which parts to absorb
    #[arg(long, short)]
    interactive: bool,

    /// Specify diff editor to be used (implies --interactive)
    #[arg(long, value_name = "NAME")]
    #[arg(add = ArgValueCandidates::new(complete::diff_editors))]
    tool: Option<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_absorb(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &AbsorbArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;

    let source_commit = workspace_command.resolve_single_rev(ui, &args.from).await?;
    let destinations = workspace_command
        .parse_union_revsets(ui, &args.into)?
        .resolve()?;

    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    let repo = workspace_command.repo().as_ref();
    let source = AbsorbSource::from_commit(repo, source_commit.clone()).await?;

    print_unmatched_explicit_paths(
        ui,
        &workspace_command,
        &fileset_expression,
        [&source_commit.tree()],
    )?;

    let diff_selector =
        workspace_command.diff_selector(ui, args.tool.as_deref(), args.interactive)?;
    let right_tree = if diff_selector.is_interactive() {
        let parent_tree = source.parent_tree().clone();
        let source_tree = source.commit().tree();
        let format_instructions = || {
            formatdoc! {"
                You are selecting changes from: {source} to be considered for
                absorption into ancestors.

                The left side of the diff shows the parent commit. The right side
                initially shows the contents of the commit you're absorbing from.

                Adjust the right side until the diff shows the changes you want to
                absorb. Selected hunks will be automatically assigned to the closest
                ancestor where the corresponding lines were last modified (using
                annotation). Hunks that cannot be assigned unambiguously will remain
                in the source commit.
                ",
                source = workspace_command.format_commit_summary(source.commit()),
            }
        };
        let selected_tree = diff_selector
            .select(
                ui,
                Diff::new(&parent_tree, &source_tree),
                Diff::new(
                    source.commit().parents_conflict_label().await?,
                    source.commit().conflict_label(),
                ),
                &matcher,
                format_instructions,
            )
            .await?;
        if selected_tree.tree_ids() == parent_tree.tree_ids() {
            return Err(user_error("No changes selected"));
        }
        selected_tree
    } else {
        source.commit().tree()
    };

    let selected_trees =
        split_hunks_to_trees(repo, &source, &right_tree, &destinations, &matcher).await?;

    let path_converter = workspace_command.path_converter();
    for (path, reason) in selected_trees.skipped_paths {
        let ui_path = path_converter.format_file_path(&path);
        writeln!(ui.warning_default(), "Skipping {ui_path}: {reason}")?;
    }

    workspace_command
        .check_rewritable(selected_trees.target_commits.keys())
        .await?;

    let mut tx = workspace_command.start_transaction();
    let stats = absorb_hunks(tx.repo_mut(), &source, selected_trees.target_commits).await?;

    if let Some(mut formatter) = ui.status_formatter() {
        if !stats.rewritten_destinations.is_empty() {
            writeln!(
                formatter,
                "Absorbed changes into {} revisions:",
                stats.rewritten_destinations.len()
            )?;
            print_updated_commits(
                formatter.as_mut(),
                &tx.commit_summary_template(),
                stats.rewritten_destinations.iter().rev(),
            )?;
        }
        if stats.num_rebased > 0 {
            writeln!(
                formatter,
                "Rebased {} descendant commits.",
                stats.num_rebased
            )?;
        }
    }

    tx.finish(
        ui,
        format!(
            "absorb changes into {} commits",
            stats.rewritten_destinations.len()
        ),
    )
    .await?;

    if let Some(mut formatter) = ui.status_formatter()
        && let Some(commit) = &stats.rewritten_source
    {
        let repo = workspace_command.repo().as_ref();
        if !commit.is_empty(repo).await? {
            writeln!(formatter, "Remaining changes:")?;
            let diff_renderer = workspace_command.diff_renderer(vec![DiffFormat::Summary]);
            let matcher = &EverythingMatcher; // also print excluded paths
            let width = ui.term_width();
            diff_renderer
                .show_patch(ui, formatter.as_mut(), commit, matcher, width)
                .await?;
        }
    }
    Ok(())
}
