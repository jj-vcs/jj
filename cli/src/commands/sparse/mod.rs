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

use std::fmt::Write as _;

use clap::Subcommand;
use itertools::Itertools as _;
use jj_lib::fileset;
use jj_lib::fileset::FilePattern;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use tracing::instrument;

use self::edit::SparseEditArgs;
use self::edit::cmd_sparse_edit;
use self::list::SparseListArgs;
use self::list::cmd_sparse_list;
use self::reset::SparseResetArgs;
use self::reset::cmd_sparse_reset;
use self::set::SparseSetArgs;
use self::set::cmd_sparse_set;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::print_checkout_stats;
use crate::command_error::CommandError;
use crate::command_error::internal_error_with_message;
use crate::command_error::print_parse_diagnostics;
use crate::ui::Ui;

/// Manage which paths from the working-copy commit are present in the working copy
///
/// The sparse patterns are a fileset expression (see `jj help -k filesets`).
/// They're displayed and edited as a list of rules, which are applied in order
/// starting from no files. Each rule either adds (`+`) or removes (`-`) the
/// files matched by a fileset expression. Paths in the rules are relative to
/// the workspace root. For example, these rules include everything except the
/// `build/` directory, but do include `build/config/`:
///
/// ```text
/// + all()
/// - build
/// + build/config
/// ```
///
/// The `sparse()` fileset function refers to the current sparse patterns.
#[derive(Subcommand, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub(crate) enum SparseCommand {
    Edit(SparseEditArgs),
    List(SparseListArgs),
    Reset(SparseResetArgs),
    Set(SparseSetArgs),
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
    }
}

async fn update_sparse_patterns_with(
    ui: &mut Ui,
    workspace_command: &mut WorkspaceCommandHelper,
    f: impl FnOnce(&mut Ui, &FilesetExpression) -> Result<FilesetExpression, CommandError>,
) -> Result<(), CommandError> {
    let (mut locked_ws, wc_commit) = workspace_command.start_working_copy_mutation().await?;
    let new_patterns = f(ui, locked_ws.locked_wc().sparse_patterns()?)?;
    let stats = locked_ws
        .locked_wc()
        .set_sparse_patterns(new_patterns)
        .await
        .map_err(|err| internal_error_with_message("Failed to update working copy paths", err))?;
    let operation_id = locked_ws.locked_wc().old_operation_id().clone();
    locked_ws.finish(operation_id).await?;
    print_checkout_stats(ui, &stats, &wc_commit)?;
    Ok(())
}

/// Parses fileset expressions specified by command arguments.
fn parse_filesets(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    values: &[String],
) -> Result<Vec<FilesetExpression>, CommandError> {
    let mut diagnostics = FilesetDiagnostics::new();
    let context = workspace_command.fileset_parse_context();
    let expressions = values
        .iter()
        .map(|value| fileset::parse_maybe_bare(&mut diagnostics, value, &context))
        .try_collect()?;
    print_parse_diagnostics(ui, "In fileset expression", &diagnostics)?;
    Ok(expressions)
}

/// Whether a sparse rule adds or removes files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SparseRuleKind {
    Include,
    Exclude,
}

impl SparseRuleKind {
    fn marker(self) -> char {
        match self {
            Self::Include => '+',
            Self::Exclude => '-',
        }
    }
}

/// Splits the sparse patterns into rules that produce the same patterns when
/// passed to `apply_sparse_rule()` in order, starting from `none()`.
fn sparse_patterns_to_rules(
    expression: &FilesetExpression,
) -> Vec<(SparseRuleKind, &FilesetExpression)> {
    let mut rules = vec![];
    collect_sparse_rules(expression, &mut rules);
    rules
}

fn collect_sparse_rules<'a>(
    expression: &'a FilesetExpression,
    rules: &mut Vec<(SparseRuleKind, &'a FilesetExpression)>,
) {
    match expression {
        FilesetExpression::None => {}
        FilesetExpression::UnionAll(expressions) => {
            let (first, rest) = expressions
                .split_first()
                .expect("union should have operands");
            collect_sparse_rules(first, rules);
            rules.extend(rest.iter().map(|expr| (SparseRuleKind::Include, expr)));
        }
        FilesetExpression::Difference(lhs, rhs) => {
            collect_sparse_rules(lhs, rules);
            rules.push((SparseRuleKind::Exclude, rhs));
        }
        _ => rules.push((SparseRuleKind::Include, expression)),
    }
}

/// Adds the files matched by `expression` to, or removes them from, the
/// `patterns`.
fn apply_sparse_rule(
    patterns: FilesetExpression,
    kind: SparseRuleKind,
    expression: FilesetExpression,
) -> FilesetExpression {
    if matches_everything(&expression) {
        return match kind {
            SparseRuleKind::Include => expression,
            SparseRuleKind::Exclude => FilesetExpression::none(),
        };
    }
    match (kind, patterns) {
        (SparseRuleKind::Include, FilesetExpression::None) => expression,
        (SparseRuleKind::Include, FilesetExpression::UnionAll(mut expressions)) => {
            expressions.push(expression);
            FilesetExpression::UnionAll(expressions)
        }
        (SparseRuleKind::Include, patterns) => {
            FilesetExpression::UnionAll(vec![patterns, expression])
        }
        (SparseRuleKind::Exclude, patterns) => patterns.difference(expression),
    }
}

fn matches_everything(expression: &FilesetExpression) -> bool {
    match expression {
        FilesetExpression::All => true,
        FilesetExpression::Pattern(FilePattern::PrefixPath(path)) => path.is_root(),
        _ => false,
    }
}

/// Formats the sparse patterns as rules, one per line.
fn format_sparse_rules(expression: &FilesetExpression) -> String {
    let mut text = String::new();
    for (kind, expression) in sparse_patterns_to_rules(expression) {
        let expression = fileset::format_workspace_relative(expression);
        writeln!(text, "{} {expression}", kind.marker()).unwrap();
    }
    text
}

#[cfg(test)]
mod tests {
    use jj_lib::fileset::FilesetAliasesMap;
    use jj_lib::fileset::FilesetParseContext;
    use jj_lib::ui_path::RepoPathUiConverter;

    use super::*;

    fn parse(text: &str) -> FilesetExpression {
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                cwd: "/ws".into(),
                base: "/ws".into(),
            },
            working_copy: None,
        };
        fileset::parse(&mut FilesetDiagnostics::new(), text, &context).unwrap()
    }

    fn apply_rules(rules: &[(SparseRuleKind, &FilesetExpression)]) -> FilesetExpression {
        rules
            .iter()
            .fold(FilesetExpression::none(), |patterns, &(kind, expr)| {
                apply_sparse_rule(patterns, kind, expr.clone())
            })
    }

    #[test]
    fn test_sparse_rules_round_trip() {
        let format = |text: &str| {
            let expression = parse(text);
            let rules = sparse_patterns_to_rules(&expression);
            assert_eq!(apply_rules(&rules), expression, "{text:?}");
            format_sparse_rules(&expression)
        };

        insta::assert_snapshot!(format("none()"), @"");
        insta::assert_snapshot!(format("all()"), @"+ all()");
        insta::assert_snapshot!(format("a | b"), @r"
        + a
        + b
        ");
        insta::assert_snapshot!(format("all() ~ build | build/config"), @r"
        + all()
        - build
        + build/config
        ");
        insta::assert_snapshot!(format("(a ~ b) & c | d"), @"
        + a ~ b & c
        + d
        ");
        insta::assert_snapshot!(format("~build"), @r"
        + all()
        - build
        ");
        insta::assert_snapshot!(format("a ~ (b | c) ~ d"), @r"
        + a
        - b | c
        - d
        ");
    }
}
