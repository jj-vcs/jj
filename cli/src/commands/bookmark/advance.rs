// Copyright 2026 The Jujutsu Authors
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
use jj_lib::dsl_util::ExpressionNode;
use jj_lib::iter_util::fallible_any;
use jj_lib::iter_util::fallible_find;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::revset;
use jj_lib::revset::ExpressionKind;
use jj_lib::revset::RevsetDiagnostics;

use super::is_fast_forward;
use super::warn_unmatched_local_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::print_parse_diagnostics;
use crate::command_error::user_error;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Advance the closest bookmarks to a target revision
///
/// The target `--to` defaults to `revsets.bookmark-advance-to`
/// (which defaults to `@`).
///
/// The bookmarks to advance are determined by `revsets.bookmark-advance-from`
/// (which defaults to `heads(::to & bookmarks())`).
///
/// Note that the from revset has access to `to`.
///
/// Positional bookmark name arguments can target specific bookmarks to advance
/// to the target, in this case the default from revset is ignored.
///
/// Example:
///
/// `jj bookmark advance --to x` - Does the equivalent of
/// `jj bookmark move --from 'heads(::x & bookmarks())' --to x`.
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkAdvanceArgs {
    /// Move bookmarks matching the given name patterns
    ///
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(add = ArgValueCandidates::new(complete::local_bookmarks))]
    names: Option<Vec<String>>,

    /// Move bookmarks to this revision
    ///
    /// Defaults to `revsets.bookmark-advance-to`.
    #[arg(long, short, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    to: Option<RevisionArg>,
}

pub fn cmd_bookmark_advance(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkAdvanceArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let to = if let Some(to) = &args.to {
        to.clone()
    } else {
        RevisionArg::from(
            workspace_command
                .settings()
                .get_string("revsets.bookmark-advance-to")?,
        )
    };

    let target_commit = workspace_command
        .resolve_single_rev(ui, &to)
        .map_err(|error| {
            if args.to.is_none() {
                error.hinted(
                    "`revsets.bookmark-advance-to` controls the default target. You can also \
                     specify a specific target with `--to`.",
                )
            } else {
                error
            }
        })?;

    let repo = workspace_command.repo().clone();

    let matched_bookmarks = {
        let mut bookmarks: Vec<_> = match &args.names {
            Some(texts) => {
                let name_expr = parse_union_name_patterns(ui, texts)?;
                let name_matcher = name_expr.to_matcher();
                let result = repo
                    .view()
                    .local_bookmarks_matching(&name_matcher)
                    .collect();
                warn_unmatched_local_bookmarks(ui, repo.view(), &name_expr)?;
                result
            }
            None => {
                // Get the default-from revset config, and provide `to`.
                let from_revset_str = workspace_command
                    .settings()
                    .get_string("revsets.bookmark-advance-from")?;
                let mut context = workspace_command.env().revset_parse_context();
                let commit_hex = target_commit.id().hex();
                context.local_variables.insert(
                    "to",
                    ExpressionNode {
                        kind: ExpressionKind::String(commit_hex.clone()),
                        span: pest::Span::new(commit_hex.as_str(), 0, commit_hex.len())
                            .expect("programmatic span shouldn't fail"),
                    },
                );

                let mut diags = RevsetDiagnostics::default();
                let expression = revset::parse(&mut diags, &from_revset_str, &context)?;
                print_parse_diagnostics(ui, "In revsets.bookmark-advance-from", &diags)?;

                let is_source_commit = workspace_command
                    .attach_revset_evaluator(expression)
                    .evaluate()?
                    .containing_fn();
                let is_source_ref = |target: &RefTarget| -> Result<bool, CommandError> {
                    Ok(fallible_any(target.added_ids(), &is_source_commit)?)
                };

                repo.view()
                    .local_bookmarks()
                    .filter_map(|(name, target)| {
                        is_source_ref(target)
                            .map(|matched| matched.then_some((name, target)))
                            .transpose()
                    })
                    .try_collect()?
            }
        };
        // Noop matches aren't errors, but should be excluded from stats.
        bookmarks.retain(|(_, old_target)| old_target.as_normal() != Some(target_commit.id()));
        bookmarks
    };

    if matched_bookmarks.is_empty() {
        writeln!(ui.status(), "No bookmarks to update.")?;
        return Ok(());
    }

    if let Some((name, _)) = fallible_find(
        matched_bookmarks.iter(),
        |(_, old_target)| -> Result<_, CommandError> {
            let is_ff = is_fast_forward(repo.as_ref(), old_target, target_commit.id())?;
            Ok(!is_ff)
        },
    )? {
        return Err(user_error(format!(
            "Refusing to advance bookmark backwards or sideways: {name}",
            name = name.as_symbol()
        )));
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
        write!(
            formatter,
            "Advanced {} bookmarks to ",
            matched_bookmarks.len()
        )?;
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
