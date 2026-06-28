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
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::GitRefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Forget a fetched Git ref
#[derive(clap::Args, Clone, Debug)]
pub struct GitRefForgetArgs {
    /// Forget a ref fetched from this remote
    #[arg(long = "remote", value_name = "REMOTE")]
    #[arg(add = ArgValueCandidates::new(complete::git_remotes))]
    remote: String,

    /// The fetched ref to forget
    #[arg(value_name = "REF")]
    ref_name: String,
}

pub async fn cmd_git_ref_forget(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitRefForgetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let remote_name = RemoteNameBuf::from(args.remote.as_str());
    let ref_name = GitRefNameBuf::from(args.ref_name.as_str());
    if workspace_command
        .repo()
        .view()
        .get_fetched_git_ref(remote_name.as_ref(), ref_name.as_ref())
        .is_absent()
    {
        return Err(user_error(format!(
            "No fetched Git ref {}@{}",
            ref_name.as_symbol(),
            remote_name.as_symbol()
        )));
    }

    let mut tx = workspace_command.start_transaction();
    tx.repo_mut().set_fetched_git_ref_target(
        RemoteName::new(&args.remote),
        ref_name.as_ref(),
        RefTarget::absent(),
    );
    tx.finish(
        ui,
        format!(
            "forget fetched git ref {}@{}",
            ref_name.as_symbol(),
            remote_name.as_symbol()
        ),
    )
    .await?;
    Ok(())
}
