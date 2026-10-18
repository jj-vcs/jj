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

mod edit;
mod list;
mod reset;
mod set;
mod show;

use std::fmt::Write as _;

use clap::Subcommand;
use itertools::Itertools as _;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetParseContext;
use jj_lib::fileset::FilesetRule;
use jj_lib::fileset::FilesetRuleKind;
use jj_lib::fileset::FilesetRules;
use tracing::instrument;

use self::edit::SparseEditArgs;
use self::edit::cmd_sparse_edit;
use self::list::SparseListArgs;
use self::list::cmd_sparse_list;
use self::reset::SparseResetArgs;
use self::reset::cmd_sparse_reset;
use self::set::SparseSetArgs;
use self::set::cmd_sparse_set;
use self::show::SparseShowArgs;
use self::show::cmd_sparse_show;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::print_checkout_stats;
use crate::command_error::CommandError;
use crate::command_error::internal_error_with_message;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Manage which paths from the working-copy commit are present in the working
/// copy
///
/// The selection is a list of rules applied in order, starting from no files.
/// Each rule adds (`+`) or removes (`-`) the files matched by a fileset
/// expression (see `jj help -k filesets`). The last rule matching a file
/// decides. For example, these rules check out everything except `build/`,
/// but do check out `build/config/`:
///
/// ```text
/// + all()
/// - build
/// + build/config
/// ```
///
/// `jj sparse show` prints the rules and `jj sparse edit` edits them. In a
/// fileset, `sparse()` matches the currently selected files.
#[derive(Subcommand, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub(crate) enum SparseCommand {
    Edit(SparseEditArgs),
    List(SparseListArgs),
    Reset(SparseResetArgs),
    Set(SparseSetArgs),
    Show(SparseShowArgs),
}

#[instrument(skip_all)]
pub(crate) async fn cmd_sparse(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &SparseCommand,
) -> Result<(), CommandError> {
    match subcommand {
        SparseCommand::Edit(args) => cmd_sparse_edit(ui, command, args).await,
        SparseCommand::List(args) => cmd_sparse_list(ui, command, args).await,
        SparseCommand::Reset(args) => cmd_sparse_reset(ui, command, args).await,
        SparseCommand::Set(args) => cmd_sparse_set(ui, command, args).await,
        SparseCommand::Show(args) => cmd_sparse_show(ui, command, args).await,
    }
}

/// Checks out `new_rules`, unless the selection changed since `old_rules` was
/// read.
async fn update_sparse_patterns(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    old_rules: &FilesetRules,
    new_rules: FilesetRules,
) -> Result<(), CommandError> {
    let (mut locked_ws, wc_commit) = workspace_command.start_working_copy_mutation().await?;
    if locked_ws.locked_wc().sparse_patterns()? != old_rules {
        return Err(user_error("The sparse patterns changed concurrently")
            .hinted("Run the command again to apply it to the new patterns."));
    }
    let stats = locked_ws
        .locked_wc()
        .set_sparse_patterns(new_rules)
        .await
        .map_err(|err| internal_error_with_message("Failed to update working copy paths", err))?;
    let operation_id = locked_ws.locked_wc().old_operation_id().clone();
    locked_ws.finish(operation_id).await?;
    print_checkout_stats(ui, &stats, &wc_commit)?;
    Ok(())
}

/// Parses command arguments into rules of the given `kind`.
fn parse_fileset_rules(
    diagnostics: &mut FilesetDiagnostics,
    context: &FilesetParseContext,
    kind: FilesetRuleKind,
    values: &[String],
) -> Result<FilesetRules, CommandError> {
    values
        .iter()
        .map(|value| {
            let expression = fileset::parse_maybe_bare(diagnostics, value, context)?;
            Ok(FilesetRule { kind, expression })
        })
        .try_collect()
}

/// Formats the rules as the `+`/`-` lines shown by `jj sparse show`.
fn format_sparse_rules(rules: &FilesetRules) -> String {
    let mut text = String::new();
    for rule in rules {
        let marker = match rule.kind {
            FilesetRuleKind::Include => '+',
            FilesetRuleKind::Exclude => '-',
        };
        let expression = fileset::format_workspace_relative(&rule.expression);
        writeln!(text, "{marker} {expression}").unwrap();
    }
    text
}

/// Parses the lines written by [`format_sparse_rules()`].
fn parse_sparse_rules(
    diagnostics: &mut FilesetDiagnostics,
    context: &FilesetParseContext,
    text: &str,
) -> Result<FilesetRules, CommandError> {
    text.lines()
        .filter(|line| !line.starts_with("JJ:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut chars = line.chars();
            let kind = match chars.next() {
                Some('+') => FilesetRuleKind::Include,
                Some('-') => FilesetRuleKind::Exclude,
                _ => {
                    return Err(user_error(format!(
                        "Sparse rule must start with `+` or `-`: {line}"
                    )));
                }
            };
            let expression = fileset::parse_maybe_bare(diagnostics, chars.as_str().trim(), context)
                .map_err(|err| {
                    user_error_with_message(format!("Failed to parse sparse rule: {line}"), err)
                })?;
            Ok(FilesetRule { kind, expression })
        })
        .try_collect()
}

#[cfg(test)]
mod tests {
    use jj_lib::fileset::FilesetAliasesMap;
    use jj_lib::ui_path::RepoPathUiConverter;

    use super::*;

    #[test]
    fn test_sparse_rules_round_trip() {
        let aliases_map = FilesetAliasesMap::new();
        let context = FilesetParseContext {
            aliases_map: &aliases_map,
            path_converter: RepoPathUiConverter::root(),
            working_copy: None,
        };
        // A selection is stored and edited as text, so formatting must be the
        // exact inverse of parsing.
        let round_trip = |lines: &[&str]| {
            let mut diagnostics = FilesetDiagnostics::new();
            let rules = parse_sparse_rules(&mut diagnostics, &context, &lines.join("\n")).unwrap();
            let text = format_sparse_rules(&rules);
            let reparsed = parse_sparse_rules(&mut diagnostics, &context, &text).unwrap();
            assert_eq!(reparsed, rules, "{text:?}");
            text
        };

        insta::assert_snapshot!(round_trip(&[]), @"");
        insta::assert_snapshot!(round_trip(&["+ all()"]), @"+ all()");
        insta::assert_snapshot!(round_trip(&["+ all()", "- build", "+ build/config"]), @r"
        + all()
        - build
        + build/config
        ");
        // Leading/trailing space, blank lines, and comments are insignificant
        insta::assert_snapshot!(round_trip(&["  -   a  ", "", "JJ: comment", "+b"]), @r"
        - a
        + b
        ");
        // Each rule keeps its own expression rather than being merged
        insta::assert_snapshot!(round_trip(&["+ a | b", "+ c & ~d"]), @r"
        + a | b
        + c & ~d
        ");
    }

    #[test]
    fn test_sparse_rules_reject_unmarked_lines() {
        let aliases_map = FilesetAliasesMap::new();
        let context = FilesetParseContext {
            aliases_map: &aliases_map,
            path_converter: RepoPathUiConverter::root(),
            working_copy: None,
        };
        let parse = |text| {
            parse_sparse_rules(&mut FilesetDiagnostics::new(), &context, text).map(|_| ())
        };
        assert!(parse("file1").is_err());
        // A multi-byte leading character must be rejected, not panic
        assert!(parse("\u{e9}file").is_err());
        assert!(parse("+ file1 |").is_err());
        assert!(parse("+ file1").is_ok());
    }
}
