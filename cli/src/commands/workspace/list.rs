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

use clap_complete::ArgValueCandidates;
use jj_lib::repo::Repo as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::commit_templater::WorkspaceRef;
use crate::complete;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// List workspaces
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceListArgs {
    /// Render each workspace using the given template
    ///
    /// All 0-argument methods of the [`WorkspaceRef` type] are available as
    /// keywords in the template expression. See [`jj help -k templates`] for
    /// more information.
    ///
    /// The default template can be set by the `templates.workspace_list`
    /// setting.
    ///
    /// [`WorkspaceRef` type]:
    ///     https://docs.jj-vcs.dev/latest/templates/#workspaceref-type
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T')]
    #[arg(add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceListArgs,
) -> Result<(), CommandError> {
    if command.is_bare_repo() {
        let repo_loader = command.load_bare_repo()?;
        let repo = repo_loader.load_at_head().await?;
        let mut formatter = ui.stdout_formatter();
        let mut workspaces: Vec<_> = repo.view().wc_commit_ids().iter().collect();
        workspaces.sort_by_key(|(name, _)| name.as_symbol().to_string());
        let cwd = command.cwd();
        for (name, wc_commit_id) in workspaces {
            let commit = repo.store().get_commit_async(wc_commit_id).await?;
            let ws_dir = cwd.join(name.as_symbol().to_string());
            let path_str = if ws_dir.is_dir() {
                format!(
                    "{} ",
                    jj_lib::file_util::relative_path(cwd, &ws_dir).display()
                )
            } else {
                String::new()
            };
            let empty_tag = if commit.is_empty(repo.as_ref()).await? {
                " (empty)"
            } else {
                ""
            };
            let desc = if commit.description().is_empty() {
                "(no description set)".to_owned()
            } else {
                commit.description().lines().next().unwrap_or("").to_owned()
            };
            writeln!(
                formatter,
                "{}: {}{:.12} {:.12}{} {}",
                name.as_symbol(),
                path_str,
                commit.change_id(),
                wc_commit_id,
                empty_tag,
                desc
            )?;
        }
        return Ok(());
    }

    let workspace_command = command.workspace_helper(ui).await?;

    let template: TemplateRenderer<WorkspaceRef> = {
        let language = workspace_command.commit_template_language();

        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => workspace_command
                .settings()
                .get("templates.workspace_list")?,
        };

        workspace_command
            .parse_template(ui, &language, &text)?
            .labeled(["workspace_list"])
    };

    let repo = workspace_command.repo();
    let mut formatter = ui.stdout_formatter();

    for (name, wc_commit_id) in repo.view().wc_commit_ids() {
        let commit = repo.store().get_commit_async(wc_commit_id).await?;
        let ws_ref = WorkspaceRef::new(name.clone(), commit);

        template.format(&ws_ref, formatter.as_mut())?;
    }

    Ok(())
}
