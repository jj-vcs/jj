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
use jj_lib::str_util::StringExpression;

use super::warn_unmatched_local_tags;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
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
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
}

pub fn cmd_tag_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TagListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
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
            None => workspace_command.settings().get("templates.tag_list")?,
        };
        workspace_command
            .parse_template(ui, &language, &text)?
            .labeled(["tag_list"])
    };

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();

    for (name, target) in view
        .local_tags()
        .filter(|(name, _)| name_matcher.is_match(name.as_str()))
    {
        let commit_ref = CommitRef::local_only(name, target.clone());
        template.format(&commit_ref, formatter.as_mut())?;
    }

    warn_unmatched_local_tags(ui, view, &name_expr)?;
    Ok(())
}
