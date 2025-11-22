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
use itertools::Itertools as _;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::workspace_store::SimpleWorkspaceStore;
use jj_lib::workspace_store::WorkspaceStore as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Stop tracking a workspace's working-copy commit in the repo
///
/// The workspace will not be touched on disk. It can be deleted from disk
/// before or after running this command.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceForgetArgs {
    /// Names of the workspaces to forget. By default, forgets only the current
    /// workspace.
    #[arg(add = ArgValueCandidates::new(complete::workspaces))]
    workspaces: Vec<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub fn cmd_workspace_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo_path = workspace_command.repo_path().to_path_buf();

    let wss = if args.workspaces.is_empty() {
        vec![workspace_command.workspace_name().to_owned()]
    } else {
        args.workspaces.clone()
    };

    for ws in &wss {
        if workspace_command
            .repo()
            .view()
            .get_wc_commit_id(ws)
            .is_none()
        {
            return Err(user_error(format!("No such workspace: {}", ws.as_symbol())));
        }
    }

    // bundle every workspace forget into a single transaction, so that e.g.
    // undo correctly restores all of them at once.
    let mut tx = workspace_command.start_transaction();

    let workspace_store = SimpleWorkspaceStore::load(&repo_path)?;

    for ws in &wss {
        tx.repo_mut().remove_wc_commit(ws)?;

        // This is to make sure not to throw error for workspaces created before
        // this change.
        if workspace_store.exists(ws) {
            workspace_store.remove_path(ws)?;
        }
    }

    let description = if let [ws] = wss.as_slice() {
        format!("forget workspace {}", ws.as_symbol())
    } else {
        format!(
            "forget workspaces {}",
            wss.iter().map(|ws| ws.as_symbol()).join(", ")
        )
    };

    tx.finish(ui, description)?;
    Ok(())
}
