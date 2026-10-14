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

use clap_complete::ArgValueCompleter;
use jj_lib::fileset::FilesetExpression;
use tracing::instrument;

use super::SparseRuleKind;
use super::apply_sparse_rule;
use super::parse_filesets;
use super::update_sparse_patterns_with;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Update the patterns that are present in the working copy
///
/// Patterns are fileset expressions (see `jj help -k filesets`). Like in other
/// commands, paths are relative to the current directory.
///
/// For example, if all you need is the `README.md` and the `lib/` directory,
/// use `jj sparse set README.md lib`. If you then no longer need the
/// `lib/tests/` directory, use `jj sparse set --exclude lib/tests`. To add
/// patterns in a different order, use `sparse()` to refer to the current
/// patterns, e.g. `jj sparse set '(src | sparse()) ~ src/generated'`.
///
/// The new patterns are computed by starting from the current patterns (or
/// from the positional arguments, or from no files if `--clear` is given),
/// then including all `--include` patterns, then excluding all `--exclude`
/// patterns.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseSetArgs {
    /// Replace the patterns with the files matched by any of these filesets
    #[arg(
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    filesets: Vec<String>,

    /// Add the files matched by these filesets
    #[arg(
        long,
        alias = "add",
        value_name = "FILESET",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    include: Vec<String>,

    /// Remove the files matched by these filesets
    #[arg(
        long,
        alias = "remove",
        value_name = "FILESET",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    exclude: Vec<String>,

    /// Start from no files instead of the current patterns
    #[arg(long, conflicts_with = "filesets")]
    clear: bool,
}

#[instrument(skip_all)]
pub async fn cmd_sparse_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SparseSetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let replacements = parse_filesets(ui, &workspace_command, &args.filesets)?;
    let includes = parse_filesets(ui, &workspace_command, &args.include)?;
    let excludes = parse_filesets(ui, &workspace_command, &args.exclude)?;
    update_sparse_patterns_with(ui, &mut workspace_command, |_ui, old_patterns| {
        let base = if args.clear {
            FilesetExpression::none()
        } else if !replacements.is_empty() {
            FilesetExpression::union_all(replacements)
        } else {
            old_patterns.clone()
        };
        let rules = itertools::chain(
            includes
                .into_iter()
                .map(|expr| (SparseRuleKind::Include, expr)),
            excludes
                .into_iter()
                .map(|expr| (SparseRuleKind::Exclude, expr)),
        );
        Ok(rules.fold(base, |patterns, (kind, expr)| {
            apply_sparse_rule(patterns, kind, expr)
        }))
    })
    .await
}
