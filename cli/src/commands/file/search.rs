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

use std::io;
use std::io::Write as _;

use clap_complete::ArgValueCompleter;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::materialize_tree_value;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringMatcher;
use jj_lib::str_util::StringPattern;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::complete;
use crate::formatter::Formatter;
use crate::ui::Ui;

/// Search for content in files
///
/// Prints each line that matches the specified pattern, prefixed by the file
/// path. Use `--name-only` to print only the file paths.
///
/// This is an early version of the command. It does not search files
/// concurrently.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileSearchArgs {
    /// The revision to search files in
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revision: RevisionArg,

    /// The pattern to search for in a single line
    ///
    /// It is a [string pattern syntax] like `kind:pattern`.  The kind
    /// defaults to regex when omitted.
    ///
    /// If it is a glob pattern, the whole line must match the pattern,
    /// so you may want to pass something like `--pattern 'glob:*foo*'`.
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long, short, value_name = "PATTERN")]
    pattern: String,

    /// Print only the paths of files that contain a match, not the matched
    /// lines
    #[arg(long, conflicts_with = "line_number")]
    name_only: bool,

    /// Prefix each matched line with its 1-based line number within the file
    #[arg(long, short = 'n')]
    line_number: bool,

    /// Only search files matching these prefixes (instead of all files)
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::all_revision_files))]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_file_search(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileSearchArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
    let commit = workspace_command
        .resolve_single_rev(ui, &args.revision)
        .await?;
    let tree = commit.tree();
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let file_matcher = fileset_expression.to_matcher();

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let store = workspace_command.repo().store().clone();

    let pattern = if let Some((kind, pattern)) = args.pattern.split_once(':') {
        StringPattern::from_str_kind(pattern, kind)
    } else {
        StringPattern::from_str_kind(args.pattern.as_str(), "regex")
    }
    .map_err(cli_error)?;
    let pattern_matcher = pattern.to_matcher();
    // TODO: Read files concurrently (depending on backend)
    for (path, value) in tree.entries_matching(file_matcher.as_ref()) {
        let value = value?;
        let materialized =
            materialize_tree_value(store.as_ref(), &path, value, tree.labels()).await?;
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
                let content = materialized_file_value.read_all(&path).await?;
                // TODO: Make output templated
                let ui_path = workspace_command.format_file_path(&path);
                if args.name_only {
                    if pattern_matcher.match_lines(&content).next().is_some() {
                        writeln!(formatter, "{ui_path}")?;
                    }
                } else {
                    write_matches(
                        formatter.as_mut(),
                        &ui_path,
                        &content,
                        &pattern_matcher,
                        args.line_number,
                    )?;
                }
            }
            MaterializedTreeValue::Symlink { .. } => {}
            MaterializedTreeValue::FileConflict(materialized_file_value) => {
                let ui_path = workspace_command.format_file_path(&path);
                // TODO: Optionally also print the conflict side
                let mut adds = materialized_file_value.contents.adds();
                if args.name_only {
                    // Multiple blobs per file; print the path if any blob
                    // matches.
                    if adds.any(|c| pattern_matcher.match_lines(c).next().is_some()) {
                        writeln!(formatter, "{ui_path}")?;
                    }
                } else {
                    // -n numbers lines within each side independently; there
                    // is no meaningful unified line numbering across sides.
                    for content in adds {
                        write_matches(
                            formatter.as_mut(),
                            &ui_path,
                            content,
                            &pattern_matcher,
                            args.line_number,
                        )?;
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

fn write_matches(
    formatter: &mut dyn Formatter,
    ui_path: &str,
    content: &[u8],
    matcher: &StringMatcher,
    line_number: bool,
) -> io::Result<()> {
    let matches = content
        .split_inclusive(|b| *b == b'\n')
        .enumerate()
        .filter_map(|(i, line)| {
            let stripped = line.strip_suffix(b"\n").unwrap_or(line);
            matcher.is_match_bytes(stripped).then_some((i + 1, line))
        });
    for (line_no, line) in matches {
        if line_number {
            write!(formatter, "{ui_path}:{line_no}:")?;
        } else {
            write!(formatter, "{ui_path}:")?;
        }
        formatter.write_all(line)?;
        if !line.ends_with(b"\n") {
            writeln!(formatter)?;
        }
    }
    Ok(())
}
