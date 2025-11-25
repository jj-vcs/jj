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

mod edit;
mod get;
mod list;
mod path;
mod set;
mod unset;

use std::path::Path;

use itertools::Itertools as _;
use jj_lib::config::ConfigFile;
use jj_lib::config::ConfigSource;
use tracing::instrument;

use self::edit::ConfigEditArgs;
use self::edit::cmd_config_edit;
use self::get::ConfigGetArgs;
use self::get::cmd_config_get;
use self::list::ConfigListArgs;
use self::list::cmd_config_list;
use self::path::ConfigPathArgs;
use self::path::cmd_config_path;
use self::set::ConfigSetArgs;
use self::set::cmd_config_set;
use self::unset::ConfigUnsetArgs;
use self::unset::cmd_config_unset;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::config::ConfigEnv;
use crate::ui::Ui;

#[derive(clap::Args, Clone, Debug)]
#[group(id = "config_level", multiple = false, required = true)]
pub(crate) struct ConfigLevelArgs {
    /// Target the user-level config
    #[arg(long)]
    user: bool,

    /// Target the repo-level config
    #[arg(long)]
    repo: bool,

    /// Target the workspace-level config
    #[arg(long)]
    workspace: bool,
}

impl ConfigLevelArgs {
    fn get_source_kind(&self) -> Option<ConfigSource> {
        if self.user {
            Some(ConfigSource::User)
        } else if self.repo {
            Some(ConfigSource::Repo)
        } else if self.workspace {
            Some(ConfigSource::Workspace)
        } else {
            None
        }
    }

    fn config_paths<'a>(&self, config_env: &'a ConfigEnv) -> Result<Vec<&'a Path>, CommandError> {
        if self.user {
            let paths = config_env.user_config_paths().collect_vec();
            if paths.is_empty() {
                return Err(user_error("No user config path found"));
            }
            Ok(paths)
        } else if self.repo {
            config_env
                .repo_config_path()
                .map(|p| vec![p])
                .ok_or_else(|| user_error("No repo config path found"))
        } else if self.workspace {
            config_env
                .workspace_config_path()
                .map(|p| vec![p])
                .ok_or_else(|| user_error("No workspace config path found"))
        } else {
            panic!("No config_level provided")
        }
    }

    fn edit_config_file(
        &self,
        ui: &Ui,
        command: &CommandHelper,
    ) -> Result<ConfigFile, CommandError> {
        let config_env = command.config_env();
        let config = command.raw_config();
        let pick_one = |mut files: Vec<ConfigFile>, not_found_error: &str| {
            if files.len() > 1 {
                let mut choices = vec![];
                let mut formatter = ui.stderr_formatter();
                for (i, file) in files.iter().enumerate() {
                    writeln!(formatter, "{}: {}", i + 1, file.path().display())?;
                    choices.push((i + 1).to_string());
                }
                drop(formatter);
                let index =
                    ui.prompt_choice("Choose a config file (default 1)", &choices, Some(0))?;
                return Ok(files[index].clone());
            }
            files.pop().ok_or_else(|| user_error(not_found_error))
        };
        if self.user {
            pick_one(
                config_env.user_config_files(config)?,
                "No user config path found to edit",
            )
        } else if self.repo {
            pick_one(
                config_env.repo_config_files(config)?,
                "No repo config path found to edit",
            )
        } else if self.workspace {
            pick_one(
                config_env.workspace_config_files(config)?,
                "No workspace config path found to edit",
            )
        } else {
            panic!("No config_level provided")
        }
    }
}

/// Manage config options
///
/// Operates on jj configuration, which comes from the config file and
/// environment variables.
///
/// See [`jj help -k config`] to know more about file locations, supported
/// config options, and other details about `jj config`.
///
/// [`jj help -k config`]:
///     https://docs.jj-vcs.dev/latest/config/
#[derive(clap::Subcommand, Clone, Debug)]
pub(crate) enum ConfigCommand {
    #[command(visible_alias("e"))]
    Edit(ConfigEditArgs),
    #[command(visible_alias("g"))]
    Get(ConfigGetArgs),
    #[command(visible_alias("l"))]
    List(ConfigListArgs),
    #[command(visible_alias("p"))]
    Path(ConfigPathArgs),
    #[command(visible_alias("s"))]
    Set(ConfigSetArgs),
    #[command(visible_alias("u"))]
    Unset(ConfigUnsetArgs),
}

#[instrument(skip_all)]
pub(crate) fn cmd_config(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &ConfigCommand,
) -> Result<(), CommandError> {
    match subcommand {
        ConfigCommand::Edit(args) => cmd_config_edit(ui, command, args),
        ConfigCommand::Get(args) => cmd_config_get(ui, command, args),
        ConfigCommand::List(args) => cmd_config_list(ui, command, args),
        ConfigCommand::Path(args) => cmd_config_path(ui, command, args),
        ConfigCommand::Set(args) => cmd_config_set(ui, command, args),
        ConfigCommand::Unset(args) => cmd_config_unset(ui, command, args),
    }
}
