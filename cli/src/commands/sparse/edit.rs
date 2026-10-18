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

use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetParseContext;
use jj_lib::ui_path::RepoPathUiConverter;
use tracing::instrument;

use super::format_sparse_rules;
use super::parse_sparse_rules;
use super::update_sparse_patterns;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::print_parse_diagnostics;
use crate::description_util::LineNumber;
use crate::ui::Ui;

const INSTRUCTIONS: &str = r#"JJ: Lines starting with "+" add, and lines starting with "-" remove, the files
JJ: matched by a fileset expression, starting from no files. Paths are relative
JJ: to the workspace root. Lines starting with "JJ:" are ignored.
"#;

/// Start an editor to update the rules that decide which paths are present in
/// the working copy
///
/// The rules are edited in the format printed by `jj sparse show` (check out
/// `jj sparse --help`). Unlike in other commands, paths are relative to the
/// workspace root, so editing from a subdirectory leaves existing rules
/// unchanged.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseEditArgs {}

#[instrument(skip_all)]
pub async fn cmd_sparse_edit(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &SparseEditArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    workspace_command.check_working_copy_writable()?;
    let old_rules = workspace_command.working_copy().sparse_patterns()?.clone();
    let content = workspace_command
        .text_editor()?
        .edit_str(
            format_sparse_rules(&old_rules) + INSTRUCTIONS,
            Some(".jjsparse"),
            LineNumber::MIN,
        )
        .map_err(|err| err.with_name("sparse patterns"))?;

    let mut diagnostics = FilesetDiagnostics::new();
    let new_rules = {
        let context = FilesetParseContext {
            path_converter: RepoPathUiConverter::root(),
            ..workspace_command.fileset_parse_context()
        };
        parse_sparse_rules(&mut diagnostics, &context, &content)?
    };
    print_parse_diagnostics(ui, "In sparse rule", &diagnostics)?;

    // fresh snapshot time: the editor may also have changed tracked files,
    // and a new exclusion could have aborted them
    let mut workspace_command = command.workspace_helper(ui).await?;
    update_sparse_patterns(ui, &mut workspace_command, &old_rules, new_rules).await
}
