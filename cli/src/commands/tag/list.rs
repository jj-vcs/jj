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

use std::rc::Rc;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringExpression;

use super::warn_unmatched_local_tags;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::commit_ref_list;
use crate::commit_ref_list::RefListItem;
use crate::commit_ref_list::SortKey;
use crate::commit_templater::CommitRef;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// List tags.
#[derive(clap::Args, Clone, Debug)]
pub struct TagListArgs {
    /// Show tags whose local name matches
    ///
    /// By default, the specified pattern matches tag names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    pub names: Option<Vec<String>>,

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

    let name_expr = match &args.names {
        Some(texts) => parse_union_name_patterns(ui, texts)?,
        None => StringExpression::all(),
    };
    let name_matcher = name_expr.to_matcher();
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

    // TODO: include remote tags
    let mut list_items = view
        .local_tags()
        .filter(|(name, _)| name_matcher.is_match(name.as_str()))
        .map(|(name, target)| {
            let primary = CommitRef::local_only(name, target.clone());
            let tracked = vec![];
            RefListItem { primary, tracked }
        })
        .collect_vec();
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
