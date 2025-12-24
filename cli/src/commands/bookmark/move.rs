// Copyright 2020-2023 The Jujutsu Authors
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

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::iter_util::fallible_any;
use jj_lib::iter_util::fallible_find;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::str_util::StringExpression;

use super::is_fast_forward;
use super::warn_unmatched_local_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_hint;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Move existing bookmarks to target revision
///
/// If bookmark names are given, the specified bookmarks will be updated to
/// point to the target revision.
///
/// If `--from` options are given, bookmarks currently pointing to the
/// specified revisions will be updated. The bookmarks can also be filtered by
/// names.
///
/// Example: pull up the nearest bookmarks to the working-copy parent
///
/// $ jj bookmark move --from 'heads(::@- & bookmarks())' --to @-
#[derive(clap::Args, Clone, Debug)]
#[command(group(clap::ArgGroup::new("source").multiple(true).required(true)))]
pub struct BookmarkMoveArgs {
    /// Move bookmarks matching the given name patterns
    ///
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(group = "source")]
    #[arg(add = ArgValueCandidates::new(complete::local_bookmarks))]
    names: Option<Vec<String>>,

    /// Move bookmarks from the given revisions
    #[arg(long, short, group = "source", value_name = "REVSETS")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    from: Vec<RevisionArg>,

    /// Move bookmarks to this revision
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    to: RevisionArg,

    /// Allow moving bookmarks backwards or sideways
    #[arg(long, short = 'B')]
    allow_backwards: bool,
}

pub fn cmd_bookmark_move(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkMoveArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let target_commit = workspace_command.resolve_single_rev(ui, &args.to)?;
    let matched_bookmarks = {
        let is_source_ref: Box<dyn Fn(&RefTarget) -> _> = if !args.from.is_empty() {
            let is_source_commit = workspace_command
                .parse_union_revsets(ui, &args.from)?
                .evaluate()?
                .containing_fn();
            Box::new(move |target| fallible_any(target.added_ids(), &is_source_commit))
        } else {
            Box::new(|_| Ok(true))
        };
        let name_expr = match &args.names {
            Some(texts) => parse_union_name_patterns(ui, texts)?,
            None => StringExpression::all(),
        };
        let name_matcher = name_expr.to_matcher();
        let mut bookmarks: Vec<_> = repo
            .view()
            .local_bookmarks_matching(&name_matcher)
            .filter_map(|(name, target)| {
                is_source_ref(target)
                    .map(|matched| matched.then_some((name, target)))
                    .transpose()
            })
            .try_collect()?;
        warn_unmatched_local_bookmarks(ui, repo.view(), &name_expr)?;
        // Noop matches aren't error, but should be excluded from stats.
        bookmarks.retain(|(_, old_target)| old_target.as_normal() != Some(target_commit.id()));
        bookmarks
    };

    if matched_bookmarks.is_empty() {
        writeln!(ui.status(), "No bookmarks to update.")?;
        return Ok(());
    }

    if !args.allow_backwards
        && let Some((name, _)) = fallible_find(
            matched_bookmarks.iter(),
            |(_, old_target)| -> Result<_, CommandError> {
                let is_ff = is_fast_forward(repo.as_ref(), old_target, target_commit.id())?;
                Ok(!is_ff)
            },
        )?
    {
        return Err(user_error_with_hint(
            format!(
                "Refusing to move bookmark backwards or sideways: {name}",
                name = name.as_symbol()
            ),
            "Use --allow-backwards to allow it.",
        ));
    }
    if target_commit.is_discardable(repo.as_ref())? {
        writeln!(ui.warning_default(), "Target revision is empty.")?;
    }

    let mut tx = workspace_command.start_transaction();
    for (name, _) in &matched_bookmarks {
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::normal(target_commit.id().clone()));
    }

    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Moved {} bookmarks to ", matched_bookmarks.len())?;
        tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
        writeln!(formatter)?;
    }
    if matched_bookmarks.len() > 1 && args.names.is_none() {
        writeln!(
            ui.hint_default(),
            "Specify bookmark by name to update just one of the bookmarks."
        )?;
    }

    tx.finish(
        ui,
        format!(
            "point bookmark {names} to commit {id}",
            names = matched_bookmarks
                .iter()
                .map(|(name, _)| name.as_symbol())
                .join(", "),
            id = target_commit.id().hex()
        ),
    )?;
    Ok(())
}
