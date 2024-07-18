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

use clap::builder::NonEmptyStringValueParser;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;

use super::has_tracked_remote_branches;
use crate::cli_util::{CommandHelper, RevisionArg};
use crate::command_error::{user_error_with_hint, CommandError};
use crate::ui::Ui;

/// Create a new branch
#[derive(clap::Args, Clone, Debug)]
pub struct BranchCreateArgs {
    /// The branch's target revision
    //
    // The `--to` alias exists for making it easier for the user to switch
    // between `branch create`, `branch move`, and `branch set`.
    #[arg(long, short, visible_alias = "to")]
    revision: Option<RevisionArg>,

    /// The branches to create
    #[arg(required = true, value_parser = NonEmptyStringValueParser::new())]
    names: Vec<String>,
}

pub fn cmd_branch_create(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BranchCreateArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let target_commit =
        workspace_command.resolve_single_rev(args.revision.as_ref().unwrap_or(&RevisionArg::AT))?;
    let view = workspace_command.repo().view();
    let branch_names = &args.names;
    for name in branch_names {
        if view.get_local_branch(name).is_present() {
            return Err(user_error_with_hint(
                format!("Branch already exists: {name}"),
                "Use `jj branch set` to update it.",
            ));
        }
        if has_tracked_remote_branches(view, name) {
            return Err(user_error_with_hint(
                format!("Tracked remote branches exist for deleted branch: {name}"),
                format!(
                    "Use `jj branch set` to recreate the local branch. Run `jj branch untrack \
                     'glob:{name}@*'` to disassociate them."
                ),
            ));
        }
    }

    let mut tx = workspace_command.start_transaction();
    for branch_name in branch_names {
        tx.mut_repo()
            .set_local_branch_target(branch_name, RefTarget::normal(target_commit.id().clone()));
    }

    if let Some(mut formatter) = ui.status_formatter() {
        write!(
            formatter,
            "Created {} branches pointing to ",
            branch_names.len()
        )?;
        tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
        writeln!(formatter)?;
    }
    if branch_names.len() > 1 && args.revision.is_none() {
        writeln!(ui.hint_default(), "Use -r to specify the target revision.")?;
    }

    tx.finish(
        ui,
        format!(
            "create branch {names} pointing to commit {id}",
            names = branch_names.join(", "),
            id = target_commit.id().hex()
        ),
    )?;
    Ok(())
}
