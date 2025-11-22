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

use clap_complete::ArgValueCandidates;
use jj_lib::file_util;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::workspace_store::SimpleWorkspaceStore;
use jj_lib::workspace_store::WorkspaceStore as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Show the workspace root directory
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceRootArgs {
    /// Name of the workspace (defaults to the current)
    #[arg(long, short, value_name = "WORKSPACE", add = ArgValueCandidates::new(complete::workspaces))]
    workspace: Option<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub fn cmd_workspace_root(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceRootArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo_path = workspace_command.repo_path().to_path_buf();

    let name = if let Some(ws_name) = &args.workspace {
        ws_name
    } else {
        workspace_command.workspace_name()
    };

    let workspace_store = SimpleWorkspaceStore::load(&repo_path)?;

    let path = if workspace_store.exists(&name.to_owned()) {
        let workspace_proto = workspace_store.get_path(&name.to_owned())?;
        dunce::canonicalize(workspace_proto.path)?
    } else if args.workspace.is_some() {
        return Err(user_error(format!(
            "No such workspace: {}",
            name.as_symbol()
        )));
    } else {
        workspace_command.workspace_root().to_path_buf()
    };

    let path_bytes = file_util::path_to_bytes(&path).map_err(user_error)?;
    ui.stdout().write_all(path_bytes)?;
    writeln!(ui.stdout())?;
    Ok(())
}
