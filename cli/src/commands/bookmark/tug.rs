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

use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringPattern;

use super::find_local_bookmarks;
use super::is_fast_forward;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::revset_util;
use crate::ui::Ui;

/// Move a bookmark to the latest non-empty descendant
///
/// Finds the closest bookmark on the specified revision or any of its
/// ancestors, then moves that bookmark forward to the topologically latest
/// non-empty descendant.
///
/// This is useful for advancing a bookmark after making several commits,
/// without having to manually specify the bookmark name or target revision.
///
/// If multiple bookmarks exist on the same commit, the alphabetically first one
/// is selected.
///
/// Example: After creating commits on top of a bookmarked commit, move the
/// bookmark forward to the latest non-empty commit:
///
/// ```shell
/// $ jj bookmark tug
/// ```
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkTugArgs {
    /// Revision to start searching for bookmarks from (searches ancestors too)
    #[arg(long, short, value_name = "REVSET", default_value = "@")]
    from: RevisionArg,

    /// Allow moving the bookmark backwards or sideways
    #[arg(long, short = 'B')]
    allow_backwards: bool,
}

pub fn cmd_bookmark_tug(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkTugArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let from_commit = workspace_command.resolve_single_rev(ui, &args.from)?;

    let (bookmark_name, bookmark_commit) = {
        let ancestors_expression = workspace_command.parse_revset(
            ui,
            &RevisionArg::from(format!("::{}", from_commit.id().hex())),
        )?;
        let store = repo.store();
        let ancestors = ancestors_expression.evaluate()?;

        let ancestor_ids: Vec<_> = ancestors.iter().try_collect()?;

        let pattern = StringPattern::everything();
        let all_bookmarks: Vec<_> = repo.view().local_bookmarks_matching(&pattern).collect();

        let mut bookmark_on_ancestor = None;
        for ancestor_id in &ancestor_ids {
            let mut bookmarks_here: Vec<_> = all_bookmarks
                .iter()
                .filter(|(_, target)| target.added_ids().any(|id| id == ancestor_id))
                .collect();

            if !bookmarks_here.is_empty() {
                bookmarks_here.sort_by_key(|(name, _)| *name);

                if bookmarks_here.len() > 1 {
                    writeln!(
                        ui.warning_default(),
                        "Multiple bookmarks found on revision {}: {}",
                        ancestor_id.hex(),
                        bookmarks_here
                            .iter()
                            .map(|(name, _)| name.as_symbol())
                            .join(", ")
                    )?;
                    writeln!(
                        ui.hint_default(),
                        "Using bookmark: {}",
                        bookmarks_here[0].0.as_symbol()
                    )?;
                }

                let (name, _) = bookmarks_here[0];
                let name_string = name.as_symbol().to_string();
                bookmark_on_ancestor = Some((name_string, store.get_commit(ancestor_id)?));
                break;
            }
        }

        bookmark_on_ancestor.ok_or_else(|| {
            user_error(format!(
                "No bookmarks found on {} or its ancestors",
                from_commit.id().hex()
            ))
        })?
    };

    let target_commit = {
        let revset_expression = workspace_command.parse_revset(
            ui,
            &RevisionArg::from(format!("heads({}+ & ~empty())", bookmark_commit.id().hex())),
        )?;
        let store = repo.store();
        let descendants = revset_expression.evaluate()?;

        let commit_ids: Vec<_> = descendants.iter().try_collect()?;

        commit_ids
            .into_iter()
            .map(|id| store.get_commit(&id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .next()
            .ok_or_else(|| {
                user_error(format!(
                    "No non-empty descendants found for revision {}",
                    bookmark_commit.id().hex()
                ))
            })?
    };

    if !args.allow_backwards {
        let matches =
            find_local_bookmarks(repo.view(), &[StringPattern::exact(bookmark_name.clone())])?;
        if let Some((name, _old_target)) = matches
            .into_iter()
            .find(|(_, old_target)| !is_fast_forward(repo.as_ref(), old_target, target_commit.id()))
        {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to move bookmark backwards or sideways: {}",
                    name.as_symbol()
                ),
                "Use --allow-backwards to allow it.",
            ));
        }
    }

    let bookmark_ref_name = revset_util::parse_bookmark_name(&bookmark_name).map_err(|e| {
        user_error(format!(
            "Failed to parse bookmark name '{bookmark_name}': {e}"
        ))
    })?;

    let mut tx = workspace_command.start_transaction();
    tx.repo_mut().set_local_bookmark_target(
        &bookmark_ref_name,
        RefTarget::normal(target_commit.id().clone()),
    );

    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Moved bookmark {bookmark_name} to ")?;
        tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
        writeln!(formatter)?;
    }

    tx.finish(
        ui,
        format!(
            "point bookmark {} to commit {}",
            bookmark_name,
            target_commit.id().hex()
        ),
    )?;
    Ok(())
}
