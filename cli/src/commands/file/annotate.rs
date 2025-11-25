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
use jj_lib::annotate::FileAnnotation;
use jj_lib::annotate::FileAnnotator;
use jj_lib::annotate::LineOrigin;
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commit_templater::AnnotationLine;
use crate::complete;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Show the source change for each line of the target file.
///
/// Annotates a revision line by line. Each line includes the source change that
/// introduced the associated line. A path to the desired file must be provided.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileAnnotateArgs {
    /// the file to annotate
    #[arg(
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    path: String,
    /// an optional revision to start at
    #[arg(
        long,
        short,
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revision: Option<RevisionArg>,
    /// Render each line using the given template
    ///
    /// All 0-argument methods of the [`AnnotationLine` type] are available as
    /// keywords in the template expression. See [`jj help -k templates`] for
    /// more information.
    ///
    /// If not specified, this defaults to the `templates.file_annotate`
    /// setting.
    ///
    /// [`AnnotationLine` type]:
    ///     https://docs.jj-vcs.dev/latest/templates/#annotationline-type
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_file_annotate(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileAnnotateArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let starting_commit = workspace_command
        .resolve_single_rev(ui, args.revision.as_ref().unwrap_or(&RevisionArg::AT))?;
    let file_path = workspace_command.parse_file_path(&args.path)?;
    let file_value = starting_commit.tree().path_value(&file_path)?;
    let ui_path = workspace_command.format_file_path(&file_path);
    if file_value.is_absent() {
        return Err(user_error(format!("No such path: {ui_path}")));
    }
    if file_value.is_tree() {
        return Err(user_error(format!(
            "Path exists but is not a regular file: {ui_path}"
        )));
    }

    let template_text = match &args.template {
        Some(value) => value.clone(),
        None => workspace_command
            .settings()
            .get_string("templates.file_annotate")?,
    };
    let language = workspace_command.commit_template_language();
    let template = workspace_command.parse_template(ui, &language, &template_text)?;

    // TODO: Should we add an option to limit the domain to e.g. recent commits?
    // Note that this is probably different from "--skip REVS", which won't
    // exclude the revisions, but will ignore diffs in those revisions as if
    // ancestor revisions had new content.
    let mut annotator = FileAnnotator::from_commit(&starting_commit, &file_path)?;
    annotator.compute(repo.as_ref(), &RevsetExpression::all())?;
    let annotation = annotator.to_annotation();

    render_file_annotation(repo.as_ref(), ui, &template, &annotation)?;
    Ok(())
}

fn render_file_annotation(
    repo: &dyn Repo,
    ui: &mut Ui,
    template_render: &TemplateRenderer<AnnotationLine>,
    annotation: &FileAnnotation,
) -> Result<(), CommandError> {
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let mut last_id = None;
    // At least in cases where the repository was jj-initialized shallowly,
    // then unshallow'd with git, some changes will not have a commit id
    // because jj does not import the unshallow'd commits. So we default
    // to the root commit id for now.
    let default_line_origin = LineOrigin {
        commit_id: repo.store().root_commit_id().clone(),
        line_number: 0,
    };
    for (line_number, (line_origin, content)) in annotation.line_origins().enumerate() {
        let line_origin = line_origin.unwrap_or(&default_line_origin);
        let commit = repo.store().get_commit(&line_origin.commit_id)?;
        let first_line_in_hunk = last_id != Some(&line_origin.commit_id);
        let annotation_line = AnnotationLine {
            commit,
            content: content.to_owned(),
            line_number: line_number + 1,
            original_line_number: line_origin.line_number + 1,
            first_line_in_hunk,
        };
        template_render.format(&annotation_line, formatter.as_mut())?;
        last_id = Some(&line_origin.commit_id);
    }

    Ok(())
}
