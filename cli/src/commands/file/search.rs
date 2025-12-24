// Copyright 2025 The Jujutsu Authors
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

use clap_complete::ArgValueCompleter;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::materialize_tree_value;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringPattern;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::complete;
use crate::ui::Ui;

/// Search for content in files
///
/// Lists files containing the specified pattern.
///
/// This is an early version of the command. It only supports glob matching for
/// now, it doesn't search files concurrently, and it doesn't indicate where in
/// the file the match was found.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileSearchArgs {
    /// The revision to search files in
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revision: RevisionArg,

    /// The glob pattern to search for
    ///
    /// The whole line must match the pattern, so you may want to pass something
    /// like `--pattern '*foo*'`.
    #[arg(long, short, value_name = "PATTERN")]
    pattern: String,

    /// Only search files matching these prefixes (instead of all files)
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_file_search(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileSearchArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let commit = workspace_command.resolve_single_rev(ui, &args.revision)?;
    let tree = commit.tree();
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let file_matcher = fileset_expression.to_matcher();

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let store = workspace_command.repo().store().clone();

    // TODO: Support other patterns than glob
    let pattern = StringPattern::glob(&args.pattern).map_err(|err| cli_error(err.to_string()))?;
    let pattern_matcher = pattern.to_matcher();
    // TODO: Read files concurrently (depending on backend)
    for (path, value) in tree.entries_matching(file_matcher.as_ref()) {
        let value = value?;
        let materialized =
            materialize_tree_value(store.as_ref(), &path, value, tree.labels()).block_on()?;
        match materialized {
            MaterializedTreeValue::Absent => panic!("Entry for absent path in file listing"),
            MaterializedTreeValue::AccessDenied(error) => {
                let ui_path = workspace_command.format_file_path(&path);
                writeln!(
                    ui.warning_default(),
                    "Skipping '{ui_path}' due to permission error: {error}"
                )?;
            }
            MaterializedTreeValue::File(mut materialized_file_value) => {
                let content = materialized_file_value.read_all(&path).block_on()?;
                // TODO: Make output templated
                let ui_path = workspace_command.format_file_path(&path);
                if let Some(_line) = pattern_matcher.match_lines(&content).next() {
                    // TODO: Optionally also print the line and line number
                    writeln!(formatter, "{ui_path}")?;
                }
            }
            MaterializedTreeValue::Symlink { .. } => {}
            MaterializedTreeValue::FileConflict(materialized_file_value) => {
                let ui_path = workspace_command.format_file_path(&path);
                for content in materialized_file_value.contents.adds() {
                    if let Some(_line) = pattern_matcher.match_lines(content).next() {
                        // TODO: Optionally also print the conflict side, line and line number
                        writeln!(formatter, "{ui_path}")?;
                        break;
                    }
                }
            }
            MaterializedTreeValue::OtherConflict { .. } => {}
            MaterializedTreeValue::GitSubmodule(_) => {}
            MaterializedTreeValue::Tree(_) => panic!("Entry for tree in file listing"),
        }
    }
    print_unmatched_explicit_paths(ui, &workspace_command, &fileset_expression, [&tree])?;
    Ok(())
}
