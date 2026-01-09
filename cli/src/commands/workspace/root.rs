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
use crate::command_error::user_error_with_message;
use crate::complete;
use crate::ui::Ui;

/// Show the workspace root directory
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceRootArgs {
    /// Name of the workspace (defaults to current)
    #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(complete::workspaces))]
    name: Option<WorkspaceNameBuf>,
}

#[instrument(skip_all)]
pub fn cmd_workspace_root(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceRootArgs,
) -> Result<(), CommandError> {
    let path = if let Some(ws_name) = &args.name {
        let workspace_command = command.workspace_helper_no_snapshot(ui)?;
        let workspace_store = SimpleWorkspaceStore::load(workspace_command.repo_path())?;

        if workspace_command
            .repo()
            .view()
            .wc_commit_ids()
            .contains_key(ws_name)
        {
            let path = workspace_store
                .get_workspace_path(ws_name)?
                .ok_or_else(|| {
                    user_error(format!(
                        "Workspace has no recorded path: {}",
                        ws_name.as_symbol()
                    ))
                })?;
            let full_path = workspace_command.repo_path().join(path);
            dunce::canonicalize(&full_path).map_err(|err| {
                user_error_with_message(
                    format!(
                        "Cannot resolve absolute workspace path: {}",
                        full_path.display()
                    ),
                    err,
                )
            })?
        } else {
            return Err(user_error(format!(
                "No such workspace: {}",
                ws_name.as_symbol()
            )));
        }
    } else {
        command.workspace_loader()?.workspace_root().into()
    };

    let path_bytes = file_util::path_to_bytes(&path).map_err(user_error)?;
    ui.stdout().write_all(path_bytes)?;
    writeln!(ui.stdout())?;
    Ok(())
}
