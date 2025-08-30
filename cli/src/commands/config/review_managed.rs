// Copyright 2025 The Jujutsu Authors
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

use std::path::PathBuf;

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error_with_message;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::config::maybe_read;
use crate::merge_tools::make_diff_sections;
use crate::ui::Ui;

/// Reviews and updates configuration stored in version control.
/// You should never need to run this command unless jj tells you to.
/// This command needs to be run when the config checked in to the repo is
/// changed, and allows you to approve or reject said changes on a line-by-line
/// basis.
#[derive(clap::Args, Clone, Debug)]
pub struct ConfigReviewManagedArgs {
    /// Trust the repository's config and skip review of it.
    /// Use this when you absolutely trust the repo config (eg. you're the only
    /// contributor).
    #[arg(long)]
    trust: bool,
}

#[instrument(skip_all)]
pub fn cmd_review_managed(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConfigReviewManagedArgs,
) -> Result<(), CommandError> {
    // It'd be nice if we were able to just error out here.
    // But in the event that a user disables their repo-managed-config in their
    // repo-managed-config, that would leave no way to re-enable it easily.
    if !command
        .raw_config()
        .as_ref()
        .get("repo-managed-config.enabled")?
    {
        writeln!(ui.warning_default(), "repo-managed-config is disabled.")?;
        writeln!(
            ui.hint_default(),
            "Enable it with `jj config set <--user|--repo> repo-managed-config.enabled true`"
        )?;
    }
    if let Some(paths) = command.config_env().repo_managed_config_paths() {
        let workspace_command = command.workspace_helper(ui)?;
        let path_converter = workspace_command.path_converter();
        let vcs = maybe_read(
            &paths
                .managed
                .to_fs_path(workspace_command.workspace_root())
                .map_err(|e| {
                    internal_error_with_message("Managed path is not a valid FS path", e)
                })?,
        )?
        .unwrap_or_default();
        let config = maybe_read(&paths.config)?.unwrap_or_default();

        if vcs.is_empty() {
            // We don't need the user to review this since it's not a security issue.
            writeln!(
                ui.status(),
                "The config file has been removed from the VCS, so we have removed the local copy \
                 too."
            )?;
            std::fs::remove_file(paths.config)?;
            std::fs::remove_file(paths.last_reviewed)?;
            return Ok(());
        }

        if config == vcs {
            writeln!(ui.status(), "Your config file is already up to date")?;
            return Ok(());
        }

        let new_config = if args.trust {
            vcs.clone()
        } else {
            let sections = make_diff_sections(
                &String::from_utf8(config).map_err(|e| {
                    user_error_with_message("Currently applied config was not utf-8", e)
                })?,
                &String::from_utf8(vcs.clone()).map_err(|e| {
                    user_error_with_message("Config stored in VCS was not utf-8", e)
                })?,
            )
            .map_err(|e| internal_error_with_message("Failed to create diff sections", e))?;
            // Ideally we'd use the user's chosen diff selector, but that
            // heavily relies on jj's objects such as Tree and Store.
            let managed_path = PathBuf::from(path_converter.format_file_path(&paths.managed));
            let recorded = scm_record::Recorder::new(
                scm_record::RecordState {
                    is_read_only: false,
                    commits: vec![],
                    files: vec![scm_record::File {
                        old_path: None,
                        path: std::borrow::Cow::Borrowed(&managed_path),
                        // This doesn't do anything.
                        file_mode: scm_record::FileMode::Unix(0o777),
                        sections,
                    }],
                },
                &mut scm_record::helpers::CrosstermInput,
            )
            .run()
            .map_err(|_| user_error("Failed to select changes"))?;

            // There's always precisely one file.
            reconstruct(&recorded.files[0].sections).into_bytes()
        };
        std::fs::write(paths.config, new_config)?;
        std::fs::write(paths.last_reviewed, vcs)?;
        writeln!(ui.status(), "Updated repo config file")?;
        Ok(())
    } else {
        Err(user_error(
            "Unable to detect location of config files. Are you in a repo?",
        ))
    }
}

fn reconstruct(sections: &[scm_record::Section]) -> String {
    let mut out: Vec<&str> = Default::default();
    for section in sections {
        match section {
            scm_record::Section::Unchanged { lines } => out.extend(lines.iter().map(AsRef::as_ref)),
            scm_record::Section::Changed { lines } => {
                for line in lines {
                    if line.is_checked == (line.change_type == scm_record::ChangeType::Added) {
                        out.push(&line.line);
                    }
                }
            }
            _ => {}
        }
    }
    out.join("")
}
