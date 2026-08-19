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

use jj_lib::fileset::FilesetExpression;
use tracing::instrument;

use super::update_sparse_patterns_with;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::description_util::TextEditor;
use crate::ui::Ui;

/// Start an editor to update the patterns that are present in the working copy
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
    let old_patterns = workspace_command.working_copy().sparse_patterns()?.clone();
    let new_patterns = edit_sparse(ui, &workspace_command, &editor, &old_patterns)?;
    update_sparse_patterns_with(ui, &mut workspace_command, |_ui, _old_patterns| {
        Ok(new_patterns)
    })
    .await
}

fn edit_sparse(
    ui: &mut Ui,
    workspace_command: &WorkspaceCommandHelper,
    editor: &TextEditor,
    sparse: &FilesetExpression,
) -> Result<FilesetExpression, CommandError> {
    let content = format!("{sparse}\n");
    let edited = editor
        .edit_str(content, Some(".jjsparse"))
        .map_err(|err| err.with_name("sparse patterns"))?;

    let lines: Vec<String> = edited
        .lines()
        .filter(|line| !line.starts_with("JJ:"))
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect();

    workspace_command.parse_file_patterns(ui, &lines)
}
