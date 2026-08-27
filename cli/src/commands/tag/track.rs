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
use itertools::Itertools as _;
use jj_lib::repo::Repo as _;
use jj_lib::revset;
use jj_lib::str_util::StringExpression;

use super::resolve_trackable_remote_tags;
use super::trackable_remote_tags_matching;
use super::warn_unmatched_local_or_remote_tags;
use super::warn_unmatched_remotes;
use crate::cli_util::CommandHelper;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::complete;
use crate::revset_util::parse_name_patterns_or_remote_symbols;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Start tracking given remote tags
///
/// A tracked remote tag will be imported as a local tag of the same name.
/// Changes to it will propagate to the existing local tag on future pulls.
#[derive(clap::Args, Clone, Debug)]
pub struct TagTrackArgs {
    /// Tag name patterns or remote tag symbols to track
    ///
    /// `TAG` matches tag names using glob syntax by default. You can also use
    /// other [string pattern syntax].
    ///
    /// `TAG@REMOTE` resolves to a remote tag exactly.
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(required = true, value_name = "TAG[@REMOTE]")]
    names: Vec<String>,

    /// Remote names to track
    ///
    /// By default, the specified pattern matches remote names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// If no remote names are given, all remote tags matching the tag names
    /// will be tracked.
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long = "remote", value_name = "REMOTE")]
    // TODO: Make this skip the remotes already tracked
    #[arg(add = ArgValueCandidates::new(complete::git_remotes))]
    remotes: Option<Vec<String>>,
}

pub async fn cmd_tag_track(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TagTrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let repo = workspace_command.repo().clone();
    let view = repo.view();

    let (tag_exprs, remote_symbols) = parse_name_patterns_or_remote_symbols(ui, &args.names)?;
    // Reject mixed syntax. It is confusing if the default @<remote> or
    // user-specified --remote flag applies only to <tag> patterns.
    if !tag_exprs.is_empty() && !remote_symbols.is_empty() {
        return Err(cli_error(
            "Cannot specify both <tag> patterns and <tag>@<remote> symbols",
        ));
    } else if args.remotes.is_some() && !remote_symbols.is_empty() {
        return Err(cli_error(
            "--remote cannot be used with <tag>@<remote> symbols",
        ));
    }
    let matched_refs = if !remote_symbols.is_empty() {
        resolve_trackable_remote_tags(ui, view, &remote_symbols)?
    } else {
        let ignored_remote = default_ignored_remote_name(repo.store())
            // suppress unmatched remotes warning for default-ignored remote
            .filter(|name| view.get_remote_view(name).is_some());
        let tag_expr = StringExpression::union_all(tag_exprs);
        let remote_expr = match (&args.remotes, ignored_remote) {
            (Some(text), _) => parse_union_name_patterns(ui, text)?,
            (None, Some(ignored)) => StringExpression::exact(ignored).negated(),
            (None, None) => StringExpression::all(),
        };
        let tag_matcher = tag_expr.to_matcher();
        let remote_matcher = remote_expr.to_matcher();
        let matched_refs =
            trackable_remote_tags_matching(view, &tag_matcher, &remote_matcher).collect();
        warn_unmatched_local_or_remote_tags(ui, view, &tag_expr)?;
        warn_unmatched_remotes(ui, view, &remote_expr)?;
        matched_refs
    };

    let mut symbols = Vec::new();
    for (symbol, remote_ref) in matched_refs {
        if remote_ref.is_tracked() {
            writeln!(
                ui.warning_default(),
                "Remote tag already tracked: {symbol}",
                symbol = revset::format_remote_ref_symbol(symbol)
            )?;
        } else {
            symbols.push(symbol);
        }
    }
    let mut tx = workspace_command.start_transaction();
    for &symbol in &symbols {
        tx.repo_mut().track_remote_tag(symbol).await?;
    }
    if !symbols.is_empty() {
        writeln!(
            ui.status(),
            "Started tracking {} remote tags.",
            symbols.len()
        )?;
    }
    tx.finish(
        ui,
        format!(
            "track remote tag {}",
            symbols
                .iter()
                .map(|&symbol| revset::format_remote_ref_symbol(symbol))
                .join(", ")
        ),
    )
    .await?;
    Ok(())
}
