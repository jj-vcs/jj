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

use std::fmt::Write as _;

use camino::Utf8Path;
use itertools::Itertools as _;
use jj_lib::repo_path::RepoPathBuf;
use tracing::instrument;

use super::update_sparse_patterns_with;
use crate::cli_util::CommandHelper;
use crate::command_error::user_error_with_message;
use crate::command_error::CommandError;
use crate::description_util::TextEditor;
use crate::ui::Ui;

/// Start an editor to update the patterns that are present in the working copy
#[derive(clap::Args, Clone, Debug)]
pub struct SparseEditArgs {}

#[instrument(skip_all)]
pub fn cmd_sparse_edit(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &SparseEditArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let editor = workspace_command.text_editor()?;
    update_sparse_patterns_with(ui, &mut workspace_command, |_ui, old_patterns| {
        let mut new_patterns = edit_sparse(&editor, old_patterns)?;
        new_patterns.sort_unstable();
        new_patterns.dedup();
        Ok(new_patterns)
    })
}

fn edit_sparse(
    editor: &TextEditor,
    sparse: &[RepoPathBuf],
) -> Result<Vec<RepoPathBuf>, CommandError> {
    let mut content = String::new();
    for sparse_path in sparse {
        // Invalid path shouldn't block editing. Edited paths will be validated.
        let workspace_relative_sparse_path = sparse_path.to_fs_path_unchecked(Utf8Path::new(""));
        writeln!(&mut content, "{workspace_relative_sparse_path}").unwrap();
    }

    let content = editor
        .edit_str(content, Some(".jjsparse"))
        .map_err(|err| err.with_name("sparse patterns"))?;

    content
        .lines()
        .filter(|line| !line.starts_with("JJ:"))
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            RepoPathBuf::from_relative_path(line).map_err(|err| {
                user_error_with_message(format!("Failed to parse sparse pattern: {line}"), err)
            })
        })
        .try_collect()
}
