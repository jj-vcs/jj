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
use jj_lib::op_store::RefTarget;
use jj_lib::str_util::StringExpression;

use super::warn_unmatched_local_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Delete an existing bookmark and propagate the deletion to remotes on the
/// next push
///
/// Revisions referred to by the deleted bookmarks are not abandoned. To delete
/// revisions as well as bookmarks, use `jj abandon`. For example, `jj abandon
/// main..<bookmark>` will abandon revisions belonging to the `<bookmark>`
/// branch (relative to the `main` branch.)
///
/// If you don't want the deletion of the local bookmark to propagate to any
/// tracked remote bookmarks, use `jj bookmark forget` instead.
#[derive(clap::Args, Clone, Debug)]
#[command(group(clap::ArgGroup::new("source").multiple(true).required(true)))]
pub struct BookmarkDeleteArgs {
    /// The bookmarks to delete
    ///
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(group = "source")]
    #[arg(add = ArgValueCandidates::new(complete::local_bookmarks))]
    names: Option<Vec<String>>,
    
    /// Delete bookmarks from the given revisions
    #[arg(long, short, group = "source", value_name = "REVSETS")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    from: Vec<RevisionArg>,
}

pub async fn cmd_bookmark_delete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkDeleteArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let repo = workspace_command.repo().clone();
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
        let bookmarks: Vec<_> = repo
            .view()
            .local_bookmarks_matching(&name_matcher)
            .filter_map(|(name, target)| {
                is_source_ref(target)
                    .map(|matched| matched.then_some((name, target)))
                    .transpose()
            })
            .try_collect()?;
        warn_unmatched_local_bookmarks(ui, repo.view(), &name_expr)?;
        bookmarks
    };
    if matched_bookmarks.is_empty() {
        writeln!(ui.status(), "No bookmarks to delete.")?;
        return Ok(());
    }

    let mut tx = workspace_command.start_transaction();
    for (name, _) in &matched_bookmarks {
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::absent());
    }
    writeln!(
        ui.status(),
        "Deleted {} bookmarks.",
        matched_bookmarks.len()
    )?;
    tx.finish(
        ui,
        format!(
            "delete bookmark {}",
            matched_bookmarks
                .iter()
                .map(|(name, _)| name.as_symbol())
                .join(", ")
        ),
    )
    .await?;
    Ok(())
}
