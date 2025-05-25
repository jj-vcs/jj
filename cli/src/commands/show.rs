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

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use jj_lib::matchers::EverythingMatcher;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::ui::Ui;

/// Show commit description and changes in a revision
#[derive(clap::Args, Clone, Debug)]
#[command(mut_arg("ignore_all_space", |a| a.short('w')))]
#[command(mut_arg("ignore_space_change", |a| a.short('b')))]
pub(crate) struct ShowArgs {
    /// Show changes in this revision, compared to its parent(s)
    #[arg(
        default_value = "@",
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revision: RevisionArg,
    /// Ignored (but lets you pass `-r` for consistency with other commands)
    #[arg(short = 'r', hide = true)]
    unused_revision: bool,
    /// Render a revision using the given template
    ///
    /// You can specify arbitrary template expressions using the
    /// [built-in keywords]. See [`jj help -k templates`] for more information.
    ///
    /// [built-in keywords]:
    ///     https://jj-vcs.github.io/jj/latest/templates/#commit-keywords
    ///
    /// [`jj help -k templates`]:
    ///     https://jj-vcs.github.io/jj/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
    #[command(flatten)]
    format: DiffFormatArgs,
    /// Do not show the patch
    #[arg(long, conflicts_with = "DiffFormatArgs")]
    no_patch: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_show(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ShowArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let commit = workspace_command.resolve_single_rev(ui, &args.revision)?;
    let template_string = match &args.template {
        Some(value) => value.to_string(),
        None => workspace_command.settings().get_string("templates.show")?,
    };
    let template = workspace_command
        .parse_commit_template(ui, &template_string)?
        .labeled(["show", "commit"]);
    let diff_renderer = workspace_command.diff_renderer_for(&args.format)?;
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();
    template.format(&commit, formatter)?;
    if !args.no_patch {
        diff_renderer.show_patch(ui, formatter, &commit, &EverythingMatcher, ui.term_width())?;
    }
    Ok(())
}
