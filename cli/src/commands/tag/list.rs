// Copyright 2020-2024 The Jujutsu Authors
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

use super::warn_unmatched_local_tags;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::commit_ref_list;
use crate::commit_ref_list::RefFilterPredicates;
use crate::commit_ref_list::SortKey;
use crate::commit_templater::CommitRef;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// List tags and their targets [alias: l]
///
/// By default, a tracked remote tag will be included only if its target is
/// different from the local tag. An untracked remote tag won't be listed. For a
/// conflicted tag (both local and remote), old target revisions are preceded by
/// a "-" and new target revisions are preceded by a "+".
#[derive(clap::Args, Clone, Debug)]
pub struct TagListArgs {
    /// Show all tracked and untracked remote tags including the ones whose
    /// targets are synchronized with the local tags
    #[arg(long, short, alias = "all")]
    all_remotes: bool,

    /// Show all tracked and untracked remote tags belonging to this remote
    ///
    /// Can be combined with `--tracked` or `--conflicted` to filter the tags
    /// shown (can be repeated.)
    ///
    /// By default, the specified pattern matches remote names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(long = "remote", value_name = "REMOTE", conflicts_with = "all_remotes")]
    #[arg(hide = true)] // TODO: unhide when remote tags get stabilized (#7528)
    #[arg(add = ArgValueCandidates::new(complete::git_remotes))]
    remotes: Option<Vec<String>>,

    /// Show tracked remote tags only
    ///
    /// This omits local Git-tracking tags by default.
    #[arg(long, short, conflicts_with = "all_remotes")]
    #[arg(hide = true)] // TODO: unhide when remote tags get stabilized (#7528)
    tracked: bool,

    /// Show conflicted tags only
    #[arg(long, short, conflicts_with = "all_remotes")]
    conflicted: bool,

    /// Show tags whose local name matches
    ///
    /// By default, the specified pattern matches tag names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    pub names: Option<Vec<String>>,

    /// Show tags whose local targets are in the given revisions
    ///
    /// Note that `-r deleted_tag` will not work since `deleted_tag` wouldn't
    /// have a local target.
    #[arg(long, short, value_name = "REVSETS")]
    revisions: Option<Vec<RevisionArg>>,

    /// Render each tag using the given template
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

    /// Sort tags based on the given key (or multiple keys)
    ///
    /// Suffix the key with `-` to sort in descending order of the value (e.g.
    /// `--sort name-`). Note that when using multiple keys, the first key is
    /// the most significant.
    ///
    /// This defaults to the `ui.tag-list-sort-keys` setting.
    #[arg(long, value_name = "SORT_KEY", value_enum, value_delimiter = ',')]
    sort: Vec<SortKey>,
}

pub fn cmd_tag_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TagListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let settings = workspace_command.settings();
    let repo = workspace_command.repo();
    let view = repo.view();

    // Like cmd_git_push(), names and revisions are OR-ed.
    let name_expr = match (&args.names, &args.revisions) {
        (Some(texts), _) => parse_union_name_patterns(ui, texts)?,
        (None, Some(_)) => StringExpression::none(),
        (None, None) => StringExpression::all(),
    };
    let matched_local_targets: HashSet<_> = if let Some(revisions) = &args.revisions {
        // Match against local targets only, which is consistent with "jj git push".
        let mut expression = workspace_command.parse_union_revsets(ui, revisions)?;
        // Intersects with the set of local tag targets to minimize the lookup
        // space.
        expression.intersect_with(&RevsetExpression::tags(StringExpression::all()));
        expression.evaluate_to_commit_ids()?.try_collect()?
    } else {
        HashSet::new()
    };
    let ignored_tracked_remote = default_ignored_remote_name(repo.store());
    // --tracked implies --remote=~git by default
    let remote_expr = match (
        &args.remotes,
        args.tracked.then_some(ignored_tracked_remote).flatten(),
    ) {
        (Some(texts), _) => parse_union_name_patterns(ui, texts)?,
        (None, Some(ignored)) => StringExpression::exact(ignored).negated(),
        (None, None) => StringExpression::all(),
    };
    let template: TemplateRenderer<Rc<CommitRef>> = {
        let language = workspace_command.commit_template_language();
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => settings.get("templates.tag_list")?,
        };
        workspace_command
            .parse_template(ui, &language, &text)?
            .labeled(["tag_list"])
    };
    let sort_keys = if args.sort.is_empty() {
        settings.get_value_with("ui.tag-list-sort-keys", commit_ref_list::parse_sort_keys)?
    } else {
        args.sort.clone()
    };

    let predicates = RefFilterPredicates {
        name_matcher: name_expr.to_matcher(),
        remote_matcher: remote_expr.to_matcher(),
        matched_local_targets,
        conflicted: args.conflicted,
        include_local_only: !args.tracked && args.remotes.is_none(),
        include_synced_remotes: args.tracked || args.all_remotes || args.remotes.is_some(),
        include_untracked_remotes: !args.tracked && (args.all_remotes || args.remotes.is_some()),
    };
    let mut list_items = commit_ref_list::collect_items(view.tags(), &predicates);
    commit_ref_list::sort(repo.store(), &mut list_items, &sort_keys)?;

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    list_items
        .iter()
        .flat_map(|item| itertools::chain([&item.primary], &item.tracked))
        .try_for_each(|commit_ref| template.format(commit_ref, formatter.as_mut()))?;
    drop(formatter);

    warn_unmatched_local_tags(ui, view, &name_expr)?;
    Ok(())
}
