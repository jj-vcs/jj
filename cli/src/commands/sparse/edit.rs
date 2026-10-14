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

use itertools::Itertools as _;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use jj_lib::fileset::FilesetParseContext;
use jj_lib::ui_path::RepoPathUiConverter;
use tracing::instrument;

use super::SparseRuleKind;
use super::apply_sparse_rule;
use super::format_sparse_rules;
use super::update_sparse_patterns_with;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::print_parse_diagnostics;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::description_util::LineNumber;
use crate::description_util::TextEditor;
use crate::ui::Ui;

/// Start an editor to update the patterns that are present in the working copy
///
/// The patterns are edited as a list of rules (see `jj sparse --help`). Paths
/// are relative to the workspace root.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseEditArgs {}

#[instrument(skip_all)]
pub async fn cmd_sparse_edit(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &SparseEditArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let editor = workspace_command.text_editor()?;
    let old_patterns = workspace_command.working_copy().sparse_patterns()?;
    let content = edit_sparse(&editor, old_patterns)?;
    let workspace_root = workspace_command.workspace_root().to_owned();
    let path_converter = RepoPathUiConverter::Fs {
        cwd: workspace_root.clone(),
        base: workspace_root,
    };
    let context = FilesetParseContext {
        path_converter: &path_converter,
        ..workspace_command.fileset_parse_context()
    };
    let new_patterns = parse_sparse_rules(ui, &content, &context)?;
    update_sparse_patterns_with(ui, &mut workspace_command, |_ui, _old_patterns| {
        Ok(new_patterns)
    })
    .await
}

fn edit_sparse(editor: &TextEditor, sparse: &FilesetExpression) -> Result<String, CommandError> {
    let mut content = format_sparse_rules(sparse);
    content.push_str(
        "\
JJ: Each line adds (+) or removes (-) the files matched by a fileset expression,
JJ: starting from no files. Paths are relative to the workspace root.
JJ: Lines starting with \"JJ:\" are ignored.
",
    );
    editor
        .edit_str(content, Some(".jjsparse"), LineNumber::MIN)
        .map_err(|err| err.with_name("sparse patterns").into())
}

fn parse_sparse_rules(
    ui: &Ui,
    content: &str,
    context: &FilesetParseContext,
) -> Result<FilesetExpression, CommandError> {
    let mut diagnostics = FilesetDiagnostics::new();
    let rules: Vec<_> = content
        .lines()
        .filter(|line| !line.starts_with("JJ:"))
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (kind, text) = if let Some(text) = line.strip_prefix('+') {
                (SparseRuleKind::Include, text)
            } else if let Some(text) = line.strip_prefix('-') {
                (SparseRuleKind::Exclude, text)
            } else {
                return Err(user_error(format!(
                    "Sparse pattern must start with '+' or '-': {line}"
                )));
            };
            let expression = fileset::parse_maybe_bare(&mut diagnostics, text.trim(), context)
                .map_err(|err| {
                    user_error_with_message(format!("Failed to parse sparse pattern: {line}"), err)
                })?;
            Ok((kind, expression))
        })
        .try_collect()?;
    print_parse_diagnostics(ui, "In sparse pattern", &diagnostics)?;
    Ok(rules
        .into_iter()
        .fold(FilesetExpression::none(), |patterns, (kind, expr)| {
            apply_sparse_rule(patterns, kind, expr)
        }))
}
