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
mod gc;
mod get;
mod list;
mod path;
mod set;
mod unset;

use std::path::Path;
use std::path::PathBuf;

use jj_lib::config::ConfigFile;
use jj_lib::config::ConfigSource;
use tracing::instrument;

use self::edit::ConfigEditArgs;
use self::edit::cmd_config_edit;
use self::gc::ConfigGcArgs;
use self::gc::cmd_config_gc;
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
use crate::ui::Ui;

#[derive(clap::Args, Clone, Debug)]
#[group(id = "config_target", multiple = false, required = true)]
pub(crate) struct ConfigTargetArgs {
    /// Target the user-level config
    #[arg(long)]
    user: bool,

    /// Target the repo-level config
    #[arg(long)]
    repo: bool,

    /// Target the workspace-level config
    #[arg(long)]
    workspace: bool,

    /// Target the config file specified by the given path
    ///
    /// The path must point to a valid configuration file location recognized
    /// by Jujutsu (such as a user/repo/workspace config, a file inside a
    /// `conf.d/` directory, or a file loaded via `$JJ_CONFIG` or
    /// `--config-file`).
    ///
    /// Unlike the global `--config-file` option (which loads an extra config
    /// file when running commands), this option specifies which file to
    /// inspect, edit, or modify on disk.
    ///
    /// If the file does not exist, commands like `set` and `edit` will create
    /// it and any missing parent directories.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
}

fn path_matches(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (dunce::canonicalize(a), dunce::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

fn is_file_in_config_dir(file_path: &Path, dir_path: &Path) -> bool {
    if file_path.extension() != Some("toml".as_ref()) {
        return false;
    }
    if dir_path.is_file() {
        return false;
    }
    if let Some(parent) = file_path.parent() {
        path_matches(parent, dir_path)
    } else {
        false
    }
}

impl ConfigTargetArgs {
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

    pub(crate) fn resolve_file(
        &self,
        ui: &Ui,
        command: &CommandHelper,
    ) -> Result<Option<(&Path, ConfigSource)>, CommandError> {
        let Some(path) = &self.file else {
            return Ok(None);
        };
        let config_env = command.config_env();
        let raw_config = command.raw_config();
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            command.cwd().join(path)
        };

        // 1. Check if it matches an already loaded layer (user, system, repo,
        //    workspace, --config-file, JJ_CONFIG, etc.)
        for layer in raw_config.as_ref().layers() {
            if let Some(layer_path) = layer.path.as_ref()
                && (path_matches(layer_path, &abs_path) || path_matches(layer_path, path))
            {
                return Ok(Some((path.as_path(), layer.source)));
            }
        }

        // 2. Check repo config path (even if not yet created on disk)
        if let Ok(Some(repo_path)) = config_env.repo_config_path(ui)
            && (path_matches(&repo_path, &abs_path) || path_matches(&repo_path, path))
        {
            return Ok(Some((path.as_path(), ConfigSource::Repo)));
        }

        // 3. Check workspace config path (even if not yet created on disk)
        if let Ok(Some(workspace_path)) = config_env.workspace_config_path(ui)
            && (path_matches(&workspace_path, &abs_path) || path_matches(&workspace_path, path))
        {
            return Ok(Some((path.as_path(), ConfigSource::Workspace)));
        }

        // 4. Check user config paths (e.g. ~/.config/jj/config.toml, ~/.jjconfig.toml,
        //    or files in conf.d)
        for user_path in config_env.user_config_paths() {
            if path_matches(user_path, &abs_path) || path_matches(user_path, path) {
                return Ok(Some((path.as_path(), ConfigSource::User)));
            }
            if is_file_in_config_dir(&abs_path, user_path) {
                return Ok(Some((path.as_path(), ConfigSource::User)));
            }
        }

        // 5. Check system config paths (/etc/jj/config.toml or files in /etc/jj/conf.d)
        for sys_path in config_env.system_config_paths() {
            if path_matches(sys_path, &abs_path) || path_matches(sys_path, path) {
                return Ok(Some((path.as_path(), ConfigSource::System)));
            }
            if is_file_in_config_dir(&abs_path, sys_path) {
                return Ok(Some((path.as_path(), ConfigSource::System)));
            }
        }

        Err(user_error(format!(
            "Configuration file '{}' is not a valid jj configuration file location",
            path.display()
        ))
        .hinted(
            "Valid config locations include user configs (`~/.config/jj/config.toml` or \
             `conf.d/*.toml`), repo/workspace configs, or files loaded with global `--config-file \
             <PATH>`.",
        ))
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
        if let Some((path, source)) = self.resolve_file(ui, command)? {
            Ok(ConfigFile::load_or_empty(source, path)?)
        } else if self.user {
            pick_one(
                config_env.user_config_files(config)?,
                "No user config path found to edit",
            )
        } else if self.repo {
            pick_one(
                config_env.repo_config_files(ui, config)?,
                "No repo config path found to edit",
            )
        } else if self.workspace {
            pick_one(
                config_env.workspace_config_files(ui, config)?,
                "No workspace config path found to edit",
            )
        } else {
            panic!("No config_target provided")
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
    Gc(ConfigGcArgs),
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
pub(crate) async fn cmd_config(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &ConfigCommand,
) -> Result<(), CommandError> {
    match subcommand {
        ConfigCommand::Edit(args) => cmd_config_edit(ui, command, args).await,
        ConfigCommand::Gc(args) => cmd_config_gc(ui, command, args).await,
        ConfigCommand::Get(args) => cmd_config_get(ui, command, args).await,
        ConfigCommand::List(args) => cmd_config_list(ui, command, args).await,
        ConfigCommand::Path(args) => cmd_config_path(ui, command, args).await,
        ConfigCommand::Set(args) => cmd_config_set(ui, command, args).await,
        ConfigCommand::Unset(args) => cmd_config_unset(ui, command, args).await,
    }
}
