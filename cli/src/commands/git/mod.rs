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

mod clone;
mod colocation;
mod export;
mod fetch;
mod import;
mod init;
mod push;
mod remote;
mod root;

use std::io::Write as _;
use std::path::Path;

use clap::Subcommand;
use clap::ValueEnum;
use jj_lib::config::ConfigFile;
use jj_lib::config::ConfigSource;
use jj_lib::git;
use jj_lib::git::UnexpectedGitBackendError;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::store::Store;

use self::clone::GitCloneArgs;
use self::clone::cmd_git_clone;
use self::colocation::GitColocationCommand;
use self::colocation::cmd_git_colocation;
use self::export::GitExportArgs;
use self::export::cmd_git_export;
use self::fetch::GitFetchArgs;
use self::fetch::cmd_git_fetch;
use self::import::GitImportArgs;
use self::import::cmd_git_import;
use self::init::GitInitArgs;
use self::init::cmd_git_init;
use self::push::GitPushArgs;
use self::push::cmd_git_push;
pub use self::push::is_push_operation;
use self::remote::RemoteCommand;
use self::remote::cmd_git_remote;
use self::root::GitRootArgs;
use self::root::cmd_git_root;
use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Commands for working with Git remotes and the underlying Git repo
///
/// See this [comparison], including a [table of commands].
///
/// [comparison]:
///     https://jj-vcs.github.io/jj/latest/git-comparison/.
///
/// [table of commands]:
///     https://jj-vcs.github.io/jj/latest/git-command-table
#[derive(Subcommand, Clone, Debug)]
pub enum GitCommand {
    Clone(GitCloneArgs),
    #[command(subcommand)]
    Colocation(GitColocationCommand),
    Export(GitExportArgs),
    Fetch(GitFetchArgs),
    Import(GitImportArgs),
    Init(GitInitArgs),
    Push(GitPushArgs),
    #[command(subcommand)]
    Remote(RemoteCommand),
    Root(GitRootArgs),
}

pub fn cmd_git(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GitCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GitCommand::Clone(args) => cmd_git_clone(ui, command, args),
        GitCommand::Colocation(subcommand) => cmd_git_colocation(ui, command, subcommand),
        GitCommand::Export(args) => cmd_git_export(ui, command, args),
        GitCommand::Fetch(args) => cmd_git_fetch(ui, command, args),
        GitCommand::Import(args) => cmd_git_import(ui, command, args),
        GitCommand::Init(args) => cmd_git_init(ui, command, args),
        GitCommand::Push(args) => cmd_git_push(ui, command, args),
        GitCommand::Remote(args) => cmd_git_remote(ui, command, args),
        GitCommand::Root(args) => cmd_git_root(ui, command, args),
    }
}

pub fn maybe_add_gitignore(workspace_command: &WorkspaceCommandHelper) -> Result<(), CommandError> {
    if workspace_command.working_copy_shared_with_git() {
        std::fs::write(
            workspace_command
                .workspace_root()
                .join(".jj")
                .join(".gitignore"),
            "/*\n",
        )
        .map_err(|e| user_error_with_message("Failed to write .jj/.gitignore file", e))
    } else {
        Ok(())
    }
}

fn get_single_remote(store: &Store) -> Result<Option<RemoteNameBuf>, UnexpectedGitBackendError> {
    let mut names = git::get_all_remote_names(store)?;
    Ok(match names.len() {
        1 => names.pop(),
        _ => None,
    })
}

/// Sets repository level `trunk()` alias to the specified remote symbol.
fn write_repository_level_trunk_alias(
    ui: &Ui,
    repo_path: &Path,
    symbol: RemoteRefSymbol<'_>,
) -> Result<(), CommandError> {
    let mut file = ConfigFile::load_or_empty(ConfigSource::Repo, repo_path.join("config.toml"))?;
    file.set_value(["revset-aliases", "trunk()"], symbol.to_string())
        .expect("initial repo config shouldn't have invalid values");
    file.save()?;
    writeln!(
        ui.status(),
        "Setting the revset alias `trunk()` to `{symbol}`",
    )?;
    Ok(())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum FetchTagsMode {
    /// Always fetch all tags
    All,

    /// Only fetch tags that point to objects that are already being
    /// transmitted.
    Included,

    /// Do not fetch any tags
    None,
}

impl FetchTagsMode {
    fn as_fetch_tags(&self) -> gix::remote::fetch::Tags {
        match self {
            Self::All => gix::remote::fetch::Tags::All,
            Self::Included => gix::remote::fetch::Tags::Included,
            Self::None => gix::remote::fetch::Tags::None,
        }
    }
}
