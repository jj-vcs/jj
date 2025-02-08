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

use clap_complete::ArgValueCandidates;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::git::GitFetch;
use jj_lib::git::GitSettings;
use jj_lib::git::import_commit;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::repo::Repo;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::commands::git::get_single_remote;
use crate::complete;
use crate::git_util::GitSubprocessUi;
use crate::git_util::load_git_import_options;
use crate::ui::Ui;

/// Fetch a ref from a Git remote
///
/// If a working-copy commit gets abandoned, it will be given a new, empty
/// commit. This is true in general; it is not specific to this command.
#[derive(clap::Args, Clone, Debug)]
pub struct GitRefFetchArgs {
    /// The remote to fetch from (only named remotes are supported)
    ///
    /// This defaults to the first remote in the `git.fetch` setting. If that is
    /// not configured, and if there are multiple remotes, the remote named
    /// "origin" will be used.
    #[arg(
        long = "remote",
        value_name = "REMOTE",
        add = ArgValueCandidates::new(complete::git_remotes),
    )]
    remote: Option<String>,
    /// The ref to fetch
    #[arg(value_name = "REF")]
    ref_name: String,
}

#[tracing::instrument(skip(ui, command))]
pub async fn cmd_git_ref_fetch(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRefFetchArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let remote_name = if let Some(remote_name) = &args.remote {
        RemoteNameBuf::from(remote_name)
    } else {
        get_default_fetch_remote(ui, &workspace_command)?
    };
    let mut tx = workspace_command.start_transaction();
    do_git_fetch(ui, &mut tx, &remote_name, &args.ref_name).await?;
    tx.finish(
        ui,
        format!(
            "fetch ref {} from git remote {}",
            args.ref_name,
            remote_name.as_symbol()
        ),
    )
    .await?;
    Ok(())
}

const DEFAULT_REMOTE: &str = "origin";

fn get_default_fetch_remote(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
) -> Result<RemoteNameBuf, CommandError> {
    const KEY: &str = "git.fetch";
    let settings = workspace_command.settings();
    if let Ok(remotes) = settings.get::<Vec<String>>(KEY) {
        if let Some(remote) = remotes.first() {
            return Ok(RemoteNameBuf::from(remote));
        }
    }
    if let Some(remote) = settings.get_string(KEY).optional()? {
        Ok(RemoteNameBuf::from(remote))
    } else if let Some(remote) = get_single_remote(workspace_command.repo().store())? {
        // if nothing was explicitly configured, try to guess
        if remote != DEFAULT_REMOTE {
            writeln!(
                ui.hint_default(),
                "Fetching from the only existing remote: {}",
                remote.as_symbol()
            )?;
        }
        Ok(remote)
    } else {
        Ok(RemoteNameBuf::from(DEFAULT_REMOTE))
    }
}

async fn do_git_fetch(
    ui: &mut Ui,
    tx: &mut WorkspaceCommandTransaction<'_>,
    remote_name: &RemoteNameBuf,
    ref_name: &str,
) -> Result<(), CommandError> {
    let remote_settings = tx.settings().remote_settings()?;
    let git_settings = GitSettings::from_settings(tx.settings())?;
    let import_options = load_git_import_options(ui, &git_settings, &remote_settings)?;
    let mut git_fetch = GitFetch::new(
        tx.repo_mut(),
        git_settings.to_subprocess_options(),
        &import_options,
    )?;

    let mut callback = GitSubprocessUi::new(ui);
    let commit_id = git_fetch.fetch_and_resolve_ref(remote_name, ref_name, &mut callback, None)?;
    let (_already_imported, commit) = import_commit(tx.repo_mut(), commit_id).await?;
    if let Some(mut formatter) = ui.status_formatter() {
        write!(formatter, "Fetched {ref_name} as ")?;
        tx.write_commit_summary(formatter.as_mut(), &commit)?;
        writeln!(formatter)?;
    }

    Ok(())
}
