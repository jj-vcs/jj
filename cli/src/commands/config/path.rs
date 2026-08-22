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

use std::io::Write as _;
use std::path::PathBuf;

use itertools::Itertools as _;
use jj_lib::file_util;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::config::ConfigEnv;
use crate::ui::Ui;

/// Print the paths to the config files
///
/// A config file at that path may or may not exist.
///
/// If `--repo` or `--workspace` is specified and the config file does not
/// exist, jj will generate a new config directory for this repo/workspace and
/// print the path to the config file in that directory.
///
/// See `jj config edit` if you'd like to immediately edit a file.
#[derive(clap::Args, Clone, Debug)]
#[group(id = "config_level", multiple = false, required = true)]
pub struct ConfigPathArgs {
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

impl ConfigPathArgs {
    fn config_paths(&self, ui: &Ui, config_env: &ConfigEnv) -> Result<Vec<PathBuf>, CommandError> {
        if self.user {
            let paths = config_env
                .user_config_paths()
                .map(|p| p.to_path_buf())
                .collect_vec();
            if paths.is_empty() {
                return Err(user_error("No user config path found"));
            }
            Ok(paths)
        } else if self.repo {
            config_env
                .repo_config_path(ui)?
                .map(|p| vec![p])
                .ok_or_else(|| user_error("No repo config path found"))
        } else if self.workspace {
            config_env
                .workspace_config_path(ui)?
                .map(|p| vec![p])
                .ok_or_else(|| user_error("No workspace config path found"))
        } else {
            panic!("No config_level provided")
        }
    }
}

#[instrument(skip_all)]
pub async fn cmd_config_path(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConfigPathArgs,
) -> Result<(), CommandError> {
    for config_path in args.config_paths(ui, command.config_env())? {
        let path_bytes = file_util::path_to_bytes(&config_path).map_err(user_error)?;
        ui.stdout().write_all(path_bytes)?;
        writeln!(ui.stdout())?;
    }
    Ok(())
}
