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

use std::slice;

use clap::ArgGroup;
use clap_complete::ArgValueCompleter;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::ui::Ui;

/// Show differences between the diffs of two revisions
///
/// This is like running `jj diff -r` on each change, then comparing those
/// results. It answers: "How do the modifications introduced by revision A
/// differ from the modifications introduced by revision B?"
///
/// For example, if two changes (`A` and `B`) both add a feature on top of
/// the same parent `P` but implement it differently, `jj interdiff --from A
/// --to B` shows what one implementation adds or removes that the other
/// doesn't:
///
/// ```text
/// A (from)  B (to)       diff(P -> B)
///  \       /             :
///   \     /         =>   : (compare diffs)
///    \   /               :
///      P                 diff(P -> A)
/// ```
///
/// A common use of this command is to compare how a change has changed
/// since the last push to a remote:
///
/// ```sh
/// $ jj interdiff --from push-xyz@origin --to push-xyz
/// ```
///
/// This command is different from `jj diff --from A --to B`, which compares
/// file contents directly. `interdiff` compares what the changes do in terms of
/// their patches, rather than their file contents. This makes a difference when
/// the two revisions have different parents (`P` and `Q`): `jj diff --from A
/// --to B` will include the changes between their parents (`P` to `Q`), while
/// `jj interdiff --from A --to B` will not.
///
/// Technically, this works by rebasing `--from` (`A`) onto `--to`'s parent
/// (`Q`) and comparing the rebased revision (`A'`) to `--to` (`B`):
///
/// ```text
/// A (from)  B (to)        B (to)
/// |         |        =>   : (compare diffs)
/// P         Q             A' (A rebased onto Q)
///                         |
///                         Q
/// ```
///
/// To see the changes throughout the whole evolution of a change instead of
/// between just two revisions, use `jj evolog -p` instead.
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
#[command(group(ArgGroup::new("to_diff").args(&["from", "to"]).multiple(true).required(true)))]
#[command(mut_arg("ignore_all_space", |a| a.short('w')))]
#[command(mut_arg("ignore_space_change", |a| a.short('b')))]
pub(crate) struct InterdiffArgs {
    /// The first revision to compare (default: @)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    from: Option<RevisionArg>,

    /// The second revision to compare (default: @)
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    to: Option<RevisionArg>,

    /// Restrict the diff to these paths
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::interdiff_files))]
    paths: Vec<String>,

    #[command(flatten)]
    format: DiffFormatArgs,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_interdiff(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &InterdiffArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let from = workspace_command
        .resolve_single_rev(ui, args.from.as_ref().unwrap_or(&RevisionArg::AT))
        .await?;
    let to = workspace_command
        .resolve_single_rev(ui, args.to.as_ref().unwrap_or(&RevisionArg::AT))
        .await?;
    let repo = workspace_command.repo();
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    print_unmatched_explicit_paths(
        ui,
        &workspace_command,
        &fileset_expression,
        // We check the parent commits to account for deleted files.
        [
            &from.parent_tree(repo.as_ref()).await?,
            &from.tree(),
            &to.parent_tree(repo.as_ref()).await?,
            &to.tree(),
        ],
    )
    .await?;

    let diff_renderer = workspace_command.diff_renderer_for(&args.format)?;
    ui.request_pager();
    diff_renderer
        .show_inter_diff(
            ui,
            ui.stdout_formatter().as_mut(),
            slice::from_ref(&from),
            &to,
            matcher.as_ref(),
            ui.term_width(),
        )
        .await?;
    Ok(())
}
