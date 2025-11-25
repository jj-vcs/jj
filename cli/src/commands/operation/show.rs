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
use itertools::Itertools as _;
use jj_lib::operation::Operation;

use super::diff::show_op_diff;
use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::command_error::CommandError;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::diff_util::DiffRenderer;
use crate::diff_util::diff_formats_for_log;
use crate::graphlog::GraphStyle;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Show changes to the repository in an operation
#[derive(clap::Args, Clone, Debug)]
pub struct OperationShowArgs {
    /// Show repository changes in this operation, compared to its parent(s)
    #[arg(default_value = "@", add = ArgValueCandidates::new(complete::operations))]
    operation: String,
    /// Don't show the graph, show a flat list of modified changes
    #[arg(long, short = 'G')]
    no_graph: bool,
    /// Render the operation using the given template
    ///
    /// You can specify arbitrary template expressions using the
    /// [built-in keywords]. See [`jj help -k templates`] for more
    /// information.
    ///
    /// [built-in keywords]:
    ///     https://docs.jj-vcs.dev/latest/templates/#operation-keywords
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
    /// Show patch of modifications to changes
    ///
    /// If the previous version has different parents, it will be temporarily
    /// rebased to the parents of the new version, so the diff is not
    /// contaminated by unrelated changes.
    #[arg(long, short = 'p')]
    patch: bool,
    /// Do not show operation diff
    #[arg(long, conflicts_with_all = ["patch", "DiffFormatArgs"])]
    no_op_diff: bool,
    #[command(flatten)]
    diff_format: DiffFormatArgs,
}

pub fn cmd_op_show(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationShowArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let workspace_env = workspace_command.env();
    let repo_loader = workspace_command.workspace().repo_loader();
    let settings = workspace_command.settings();
    let op = workspace_command.resolve_single_op(&args.operation)?;
    let parent_ops: Vec<_> = op.parents().try_collect()?;
    let merged_parent_op = repo_loader.merge_operations(parent_ops.clone(), None)?;
    let parent_repo = repo_loader.load_at(&merged_parent_op)?;
    let repo = repo_loader.load_at(&op)?;

    let id_prefix_context = workspace_env.new_id_prefix_context();
    let commit_summary_template = {
        let language = workspace_env.commit_template_language(repo.as_ref(), &id_prefix_context);
        let text = settings.get_string("templates.commit_summary")?;
        workspace_env
            .parse_template(ui, &language, &text)?
            .labeled(["op_show", "commit"])
    };

    let graph_style = GraphStyle::from_settings(settings)?;
    let with_content_format = LogContentFormat::new(ui, settings)?;
    let diff_renderer = {
        let formats = diff_formats_for_log(settings, &args.diff_format, args.patch)?;
        let path_converter = workspace_env.path_converter();
        let conflict_marker_style = workspace_env.conflict_marker_style();
        (!formats.is_empty()).then(|| {
            DiffRenderer::new(
                repo.as_ref(),
                path_converter,
                conflict_marker_style,
                formats,
            )
        })
    };

    let template: TemplateRenderer<Operation> = {
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => settings.get_string("templates.op_show")?,
        };
        workspace_command
            .parse_operation_template(ui, &text)?
            .labeled(["op_show", "operation"])
    };

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    template.format(&op, formatter.as_mut())?;

    if !args.no_op_diff {
        // TODO: Merged repo may have newly rebased commits, which wouldn't exist in
        // the index. (#4465)
        if parent_ops.len() > 1 {
            return Ok(());
        }
        show_op_diff(
            ui,
            formatter.as_mut(),
            repo.as_ref(),
            &parent_repo,
            &repo,
            &commit_summary_template,
            (!args.no_graph).then_some(graph_style),
            &with_content_format,
            diff_renderer.as_ref(),
        )?;
    }
    Ok(())
}
