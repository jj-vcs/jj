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

use std::collections::HashSet;
use std::rc::Rc;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::str_util::StringExpression;

use super::warn_unmatched_local_or_remote_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::commit_ref_list;
use crate::commit_ref_list::RefListItem;
use crate::commit_ref_list::SortKey;
use crate::commit_templater::CommitRef;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// List bookmarks and their targets
///
/// By default, a tracked remote bookmark will be included only if its target is
/// different from the local target. An untracked remote bookmark won't be
/// listed. For a conflicted bookmark (both local and remote), old target
/// revisions are preceded by a "-" and new target revisions are preceded by a
/// "+".
///
/// See [`jj help -k bookmarks`] for more information.
///
/// [`jj help -k bookmarks`]:
///     https://docs.jj-vcs.dev/latest/bookmarks
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkListArgs {
    /// Show all tracked and untracked remote bookmarks including the ones
    /// whose targets are synchronized with the local bookmarks
    #[arg(long, short, alias = "all")]
    all_remotes: bool,

    /// Show all tracked and untracked remote bookmarks belonging to this remote
    ///
    /// Can be combined with `--tracked` or `--conflicted` to filter the
    /// bookmarks shown (can be repeated.)
    ///
    /// By default, the specified pattern matches remote names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long = "remote", value_name = "REMOTE", conflicts_with_all = ["all_remotes"])]
    #[arg(add = ArgValueCandidates::new(complete::git_remotes))]
    remotes: Option<Vec<String>>,

    /// Show tracked remote bookmarks only
    ///
    /// This omits local Git-tracking bookmarks by default.
    #[arg(long, short, conflicts_with_all = ["all_remotes"])]
    tracked: bool,

    /// Show conflicted bookmarks only
    #[arg(long, short, conflicts_with_all = ["all_remotes"])]
    conflicted: bool,

    /// Show bookmarks whose local name matches
    ///
    /// By default, the specified pattern matches bookmark names with glob
    /// syntax. You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(add = ArgValueCandidates::new(complete::bookmarks))]
    names: Option<Vec<String>>,

    /// Show bookmarks whose local targets are in the given revisions
    ///
    /// Note that `-r deleted_bookmark` will not work since `deleted_bookmark`
    /// wouldn't have a local target.
    #[arg(long, short, value_name = "REVSETS")]
    revisions: Option<Vec<RevisionArg>>,

    /// Render each bookmark using the given template
    ///
    /// All 0-argument methods of the [`CommitRef` type] are available as
    /// keywords in the template expression. See [`jj help -k templates`]
    /// for more information.
    ///
    /// [`CommitRef` type]:
    ///     https://docs.jj-vcs.dev/latest/templates/#commitref-type
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T')]
    #[arg(add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,

    /// Sort bookmarks based on the given key (or multiple keys)
    ///
    /// Suffix the key with `-` to sort in descending order of the value (e.g.
    /// `--sort name-`). Note that when using multiple keys, the first key is
    /// the most significant.
    ///
    /// This defaults to the `ui.bookmark-list-sort-keys` setting.
    #[arg(long, value_name = "SORT_KEY", value_enum, value_delimiter = ',')]
    sort: Vec<SortKey>,
}

pub fn cmd_bookmark_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let view = repo.view();

    // Like cmd_git_push(), names and revisions are OR-ed.
    let name_expr = match (&args.names, &args.revisions) {
        (Some(texts), _) => parse_union_name_patterns(ui, texts)?,
        (None, Some(_)) => StringExpression::none(),
        (None, None) => StringExpression::all(),
    };
    let name_matcher = name_expr.to_matcher();
    let matched_local_targets: HashSet<_> = if let Some(revisions) = &args.revisions {
        // Match against local targets only, which is consistent with "jj git push".
        let mut expression = workspace_command.parse_union_revsets(ui, revisions)?;
        // Intersects with the set of local bookmark targets to minimize the lookup
        // space.
        expression.intersect_with(&RevsetExpression::bookmarks(StringExpression::all()));
        expression.evaluate_to_commit_ids()?.try_collect()?
    } else {
        HashSet::new()
    };

    let template: TemplateRenderer<Rc<CommitRef>> = {
        let language = workspace_command.commit_template_language();
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => workspace_command
                .settings()
                .get("templates.bookmark_list")?,
        };
        workspace_command
            .parse_template(ui, &language, &text)?
            .labeled(["bookmark_list"])
    };

    let ignored_tracked_remote = default_ignored_remote_name(repo.store());
    let remote_expr = match &args.remotes {
        Some(texts) => parse_union_name_patterns(ui, texts)?,
        None => StringExpression::all(),
    };
    let remote_matcher = remote_expr.to_matcher();
    let mut bookmark_list_items: Vec<RefListItem> = Vec::new();
    let bookmarks_to_list = view
        .bookmarks()
        .filter(|(name, target)| {
            name_matcher.is_match(name.as_str())
                || target
                    .local_target
                    .added_ids()
                    .any(|id| matched_local_targets.contains(id))
        })
        .filter(|(_, target)| !args.conflicted || target.local_target.has_conflict());
    let mut any_conflicts = false;
    for (name, bookmark_target) in bookmarks_to_list {
        let local_target = bookmark_target.local_target;
        any_conflicts |= local_target.has_conflict();
        let remote_refs = bookmark_target.remote_refs;
        let (mut tracked_remote_refs, untracked_remote_refs) = remote_refs
            .iter()
            .copied()
            .filter(|(remote_name, _)| remote_matcher.is_match(remote_name.as_str()))
            .partition::<Vec<_>, _>(|&(_, remote_ref)| remote_ref.is_tracked());

        if args.tracked {
            tracked_remote_refs.retain(|&(remote, _)| {
                ignored_tracked_remote.is_none_or(|ignored| remote != ignored)
            });
        } else if !args.all_remotes && args.remotes.is_none() {
            tracked_remote_refs.retain(|&(_, remote_ref)| remote_ref.target != *local_target);
        }

        let include_local_only = !args.tracked && args.remotes.is_none();
        if include_local_only && local_target.is_present() || !tracked_remote_refs.is_empty() {
            let primary = CommitRef::local(
                name,
                local_target.clone(),
                remote_refs.iter().map(|&(_, remote_ref)| remote_ref),
            );
            let tracked = tracked_remote_refs
                .iter()
                .map(|&(remote, remote_ref)| {
                    CommitRef::remote(name, remote, remote_ref.clone(), local_target)
                })
                .collect();
            bookmark_list_items.push(RefListItem { primary, tracked });
        }

        if !args.tracked && (args.all_remotes || args.remotes.is_some()) {
            bookmark_list_items.extend(untracked_remote_refs.iter().map(
                |&(remote, remote_ref)| RefListItem {
                    primary: CommitRef::remote_only(name, remote, remote_ref.target.clone()),
                    tracked: vec![],
                },
            ));
        }
    }

    let sort_keys = if args.sort.is_empty() {
        workspace_command.settings().get_value_with(
            "ui.bookmark-list-sort-keys",
            commit_ref_list::parse_sort_keys,
        )?
    } else {
        args.sort.clone()
    };
    commit_ref_list::sort(repo.store(), &mut bookmark_list_items, &sort_keys)?;

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    bookmark_list_items
        .iter()
        .flat_map(|item| itertools::chain([&item.primary], &item.tracked))
        .try_for_each(|commit_ref| template.format(commit_ref, formatter.as_mut()))?;
    drop(formatter);

    warn_unmatched_local_or_remote_bookmarks(ui, view, &name_expr)?;

    if any_conflicts {
        writeln!(
            ui.hint_default(),
            "Some bookmarks have conflicts. Use `jj bookmark set <name> -r <rev>` to resolve."
        )?;
    }

    #[cfg(feature = "git")]
    if jj_lib::git::get_git_backend(repo.store()).is_ok() {
        // Print only one of these hints. It's not important to mention unexported
        // bookmarks, but user might wonder why deleted bookmarks are still listed.
        let deleted_tracking = bookmark_list_items
            .iter()
            .filter(|item| item.primary.is_local() && item.primary.is_absent())
            .map(|item| {
                item.tracked.iter().any(|r| {
                    let remote = r.remote_name().expect("tracked ref should be remote");
                    ignored_tracked_remote.is_none_or(|ignored| remote != ignored)
                })
            })
            .max();
        match deleted_tracking {
            Some(true) => {
                writeln!(
                    ui.hint_default(),
                    "Bookmarks marked as deleted can be *deleted permanently* on the remote by \
                     running `jj git push --deleted`. Use `jj bookmark forget` if you don't want \
                     that."
                )?;
            }
            Some(false) => {
                writeln!(
                    ui.hint_default(),
                    "Bookmarks marked as deleted will be deleted from the underlying Git repo on \
                     the next `jj git export`."
                )?;
            }
            None => {}
        }
    }

    Ok(())
}
