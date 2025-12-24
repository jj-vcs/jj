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

use std::fmt::Debug;
use std::io::Write as _;

use clap::Subcommand;
use jj_lib::backend::CommitId;
use jj_lib::backend::FileId;
use jj_lib::backend::SymlinkId;
use jj_lib::backend::TreeId;
use jj_lib::backend::TreeValue;
use jj_lib::op_store::OperationId;
use jj_lib::op_store::ViewId;
use jj_lib::repo_path::RepoPathBuf;
use pollster::FutureExt as _;
use tokio::io::AsyncReadExt as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Show information about an operation and its view
#[derive(Subcommand, Clone, Debug)]
pub enum DebugObjectArgs {
    Commit(DebugObjectCommitArgs),
    File(DebugObjectFileArgs),
    Operation(DebugObjectOperationArgs),
    Symlink(DebugObjectSymlinkArgs),
    Tree(DebugObjectTreeArgs),
    View(DebugObjectViewArgs),
}

#[derive(clap::Args, Clone, Debug)]
pub struct DebugObjectCommitArgs {
    id: String,
}

#[derive(clap::Args, Clone, Debug)]
pub struct DebugObjectFileArgs {
    #[arg(value_hint = clap::ValueHint::FilePath)]
    path: String,

    id: String,
}

#[derive(clap::Args, Clone, Debug)]
pub struct DebugObjectOperationArgs {
    id: String,
}

#[derive(clap::Args, Clone, Debug)]
pub struct DebugObjectSymlinkArgs {
    #[arg(value_hint = clap::ValueHint::FilePath)]
    path: String,

    id: String,
}

#[derive(clap::Args, Clone, Debug)]
#[command(group(clap::ArgGroup::new("target").required(true)))]
pub struct DebugObjectTreeArgs {
    #[arg(value_hint = clap::ValueHint::DirPath)]
    dir: String,

    #[arg(group = "target")]
    id: Option<String>,

    #[arg(long, short, group = "target")]
    revision: Option<RevisionArg>,
}

#[derive(clap::Args, Clone, Debug)]
#[command(group(clap::ArgGroup::new("target").required(true)))]
pub struct DebugObjectViewArgs {
    #[arg(group = "target")]
    id: Option<String>,

    #[arg(long, group = "target")]
    op: Option<String>,
}

pub fn cmd_debug_object(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &DebugObjectArgs,
) -> Result<(), CommandError> {
    // Resolve the operation without loading the repo, so this command can be used
    // even if e.g. the view object is broken.
    let workspace = command.load_workspace()?;
    let repo_loader = workspace.repo_loader();

    match args {
        DebugObjectArgs::Commit(args) => {
            let id = CommitId::try_from_hex(&args.id)
                .ok_or_else(|| user_error("Invalid hex commit id"))?;
            let commit = repo_loader.store().get_commit(&id)?;
            writeln!(ui.stdout(), "{:#?}", commit.store_commit())?;
        }
        DebugObjectArgs::File(args) => {
            let id =
                FileId::try_from_hex(&args.id).ok_or_else(|| user_error("Invalid hex file id"))?;
            let path = RepoPathBuf::from_internal_string(&args.path).map_err(user_error)?;
            let mut contents = repo_loader.store().read_file(&path, &id).block_on()?;
            let mut buf = vec![];
            contents.read_to_end(&mut buf).block_on()?;
            ui.stdout().write_all(&buf)?;
        }
        DebugObjectArgs::Operation(args) => {
            let id = OperationId::try_from_hex(&args.id)
                .ok_or_else(|| user_error("Invalid hex operation id"))?;
            let operation = repo_loader.op_store().read_operation(&id).block_on()?;
            writeln!(ui.stdout(), "{operation:#?}")?;
        }
        DebugObjectArgs::Symlink(args) => {
            let id = SymlinkId::try_from_hex(&args.id)
                .ok_or_else(|| user_error("Invalid hex symlink id"))?;
            let path = RepoPathBuf::from_internal_string(&args.path).map_err(user_error)?;
            let target = repo_loader.store().read_symlink(&path, &id).block_on()?;
            writeln!(ui.stdout(), "{target}")?;
        }
        DebugObjectArgs::Tree(args) => {
            let dir = RepoPathBuf::from_internal_string(&args.dir).map_err(user_error)?;
            let id = if let Some(rev) = &args.revision {
                let workspace_command = command.workspace_helper_no_snapshot(ui)?;
                let commit = workspace_command.resolve_single_rev(ui, rev)?;
                let tree_value = commit.tree().path_value(&dir)?;
                if let Some(Some(TreeValue::Tree(id))) = tree_value.as_resolved() {
                    id.clone()
                } else {
                    return Err(user_error("The path is not a single tree in the commit"));
                }
            } else {
                TreeId::try_from_hex(args.id.as_ref().unwrap())
                    .ok_or_else(|| user_error("Invalid hex tree id"))?
            };
            let tree = repo_loader.store().get_tree(dir, &id)?;
            writeln!(ui.stdout(), "{:#?}", tree.data())?;
        }
        DebugObjectArgs::View(args) => {
            let id = if let Some(op_string) = &args.op {
                let workspace_command = command.workspace_helper_no_snapshot(ui)?;
                let op = workspace_command.resolve_single_op(op_string)?;
                op.view_id().clone()
            } else {
                ViewId::try_from_hex(args.id.as_ref().unwrap())
                    .ok_or_else(|| user_error("Invalid hex view id"))?
            };
            let view = repo_loader.op_store().read_view(&id).block_on()?;
            writeln!(ui.stdout(), "{view:#?}")?;
        }
    }

    Ok(())
}
