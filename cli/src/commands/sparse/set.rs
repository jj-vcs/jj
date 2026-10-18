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

use std::collections::BTreeSet;

use clap_complete::ArgValueCompleter;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetRuleKind;
use jj_lib::fileset::FilesetRules;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use tracing::instrument;

use super::parse_fileset_rules;
use super::update_sparse_patterns;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::print_parse_diagnostics;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Update the paths that are present in the working copy
///
/// Patterns are fileset expressions (see `jj help -k filesets`). As in other
/// commands, paths are relative to the current directory.
///
/// For example, if all you need is the `README.md` and the `lib/` directory,
/// use `jj sparse set README.md lib`. If you then no longer need
/// `lib/tests/`, use `jj sparse set --exclude lib/tests`. `sparse()` refers
/// to the current selection, so patterns can also be combined in any other
/// order, e.g. `jj sparse set '(src | sparse()) ~ src/generated'`.
///
/// The new selection starts from the positional arguments, or from the
/// current selection if there are none (or from no files with `--clear`).
/// `--include` patterns are then applied, followed by `--exclude` patterns.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseSetArgs {
    /// Replace the selection with the files matched by these filesets
    #[arg(
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    filesets: Vec<String>,

    /// Add the files matched by these filesets to the selection
    #[arg(
        long,
        value_name = "FILESET",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    include: Vec<String>,

    /// Remove the files matched by these filesets from the selection
    #[arg(
        long,
        value_name = "FILESET",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    exclude: Vec<String>,

    /// Deprecated. Use `--include` with a fileset instead
    ///
    /// Unlike `--include`, this takes a literal path relative to the
    /// workspace root.
    // TODO: delete --add/--remove in jj 0.53 or so.
    #[arg(
        long,
        conflicts_with_all = ["filesets", "include", "exclude"],
        value_hint = clap::ValueHint::AnyPath,
        value_parser = |s: &str| RepoPathBuf::from_relative_path(s),
    )]
    add: Vec<RepoPathBuf>,

    /// Deprecated. Use `--exclude` with a fileset instead
    ///
    /// Unlike `--exclude`, this deletes an exact `--add`ed path rather than
    /// excluding the files under it.
    #[arg(
        long,
        conflicts_with_all = ["clear", "filesets", "include", "exclude"],
        value_hint = clap::ValueHint::AnyPath,
        value_parser = |s: &str| RepoPathBuf::from_relative_path(s),
    )]
    remove: Vec<RepoPathBuf>,

    /// Start from no files instead of the current selection
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
    let old_rules = workspace_command.working_copy().sparse_patterns()?.clone();

    let new_rules = if !args.add.is_empty() || !args.remove.is_empty() {
        writeln!(
            ui.warning_default(),
            "--add/--remove are deprecated; use --include/--exclude instead. Those take \
             filesets, with paths relative to the current directory."
        )?;
        legacy_path_rules(args, &old_rules)?
    } else {
        let context = workspace_command.fileset_parse_context();
        let mut diagnostics = FilesetDiagnostics::new();
        let mut parse_rules = |kind, values: &[String]| {
            parse_fileset_rules(&mut diagnostics, &context, kind, values)
        };

        let replacement = parse_rules(FilesetRuleKind::Include, &args.filesets)?;
        let mut rules = if args.clear {
            FilesetRules::none()
        } else if !replacement.is_empty() {
            replacement
        } else {
            old_rules.clone()
        };

        rules.extend(parse_rules(FilesetRuleKind::Include, &args.include)?);
        rules.extend(parse_rules(FilesetRuleKind::Exclude, &args.exclude)?);
        print_parse_diagnostics(ui, "In fileset expression", &diagnostics)?;

        rules
    };

    update_sparse_patterns(ui, &mut workspace_command, &old_rules, new_rules).await
}

/// Applies the deprecated `--add`/`--remove` flags, which operate on a sorted
/// set of literal workspace-root-relative prefixes rather than on filesets.
fn legacy_path_rules(
    args: &SparseSetArgs,
    old_rules: &FilesetRules,
) -> Result<FilesetRules, CommandError> {
    let mut paths: BTreeSet<RepoPathBuf> = if args.clear {
        BTreeSet::new()
    } else {
        old_rules
            .to_path_prefixes()
            .ok_or_else(|| {
                user_error("--add/--remove can only update a plain list of paths")
                    .hinted("Use --include/--exclude instead.")
            })?
            .into_iter()
            .map(RepoPath::to_owned)
            .collect()
    };
    paths.retain(|path| !args.remove.contains(path));
    paths.extend(args.add.iter().cloned());
    Ok(FilesetRules::from_path_prefixes(paths))
}
