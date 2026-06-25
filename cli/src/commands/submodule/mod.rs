// Copyright 2026 The Jujutsu Authors
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
use std::path::Path;

use clap_complete::ArgValueCompleter;
use jj_lib::backend::CommitId;
use jj_lib::backend::TreeValue;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::complete;
use crate::ui::Ui;

/// Manage Git submodules.
#[derive(clap::Subcommand, Clone, Debug)]
pub enum SubmoduleCommand {
    Bind(SubmoduleBindArgs),
}

pub async fn cmd_submodule(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &SubmoduleCommand,
) -> Result<(), CommandError> {
    match subcommand {
        SubmoduleCommand::Bind(args) => cmd_submodule_bind(ui, command, args).await,
    }
}

/// Explicitly update a submodule gitlink.
///
/// The submodule revision is resolved in the submodule repository at PATH. The
/// selected superproject revision is then rewritten so PATH stores that commit
/// as a `160000` Git submodule tree entry. The superproject working-copy
/// snapshot does not update submodule gitlinks automatically.
#[derive(clap::Args, Clone, Debug)]
pub struct SubmoduleBindArgs {
    /// Submodule path to update
    #[arg(value_name = "PATH", value_hint = clap::ValueHint::DirPath)]
    #[arg(add = ArgValueCompleter::new(complete::submodule_paths))]
    path: String,

    /// Revision to resolve in the submodule repository
    #[arg(long, short, value_name = "SUBMODULE_REV")]
    #[arg(add = ArgValueCompleter::new(complete::submodule_revision))]
    revision: String,

    /// Superproject revision to rewrite
    #[arg(long, value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    change: Option<RevisionArg>,
}

#[instrument(skip_all)]
async fn cmd_submodule_bind(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SubmoduleBindArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    jj_lib::git::get_git_backend(workspace_command.repo().store())?;

    let path = workspace_command.parse_file_path(&args.path)?;
    if path.is_root() {
        return Err(user_error("Cannot bind the root path as a submodule"));
    }
    let ui_path = workspace_command.format_file_path(&path);

    let target_superproject_commit = workspace_command
        .resolve_single_rev(ui, args.change.as_ref().unwrap_or(&RevisionArg::AT))
        .await?;
    workspace_command
        .check_rewritable([target_superproject_commit.id()])
        .await?;

    let old_value = target_superproject_commit.tree().path_value(&path).await?;
    let old_gitlink = inspect_existing_submodule_gitlink(&old_value, &ui_path)?;
    let submodule_fs_path = path
        .to_fs_path(workspace_command.workspace_root())
        .map_err(internal_error)?;
    let target_commit_id =
        resolve_submodule_revision(ui, command, &submodule_fs_path, &args.revision, &ui_path)
            .await?;
    if old_gitlink.as_normal() == Some(&target_commit_id) {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let mut tree_builder = MergedTreeBuilder::new(target_superproject_commit.tree());
    tree_builder.set_or_remove(
        path.clone(),
        Merge::normal(TreeValue::GitSubmodule(target_commit_id.clone())),
    );
    let new_tree = tree_builder.write_tree().await?;

    let mut tx = workspace_command.start_transaction();
    let new_commit = tx
        .repo_mut()
        .rewrite_commit(&target_superproject_commit)
        .set_tree(new_tree)
        .write()
        .await?;
    let num_rebased = tx.repo_mut().rebase_descendants().await?;
    if let Some(mut formatter) = ui.status_formatter() {
        writeln!(formatter, "Updated Git submodule {ui_path}:")?;
        match &old_gitlink {
            ExistingSubmoduleGitlink::Normal(old_commit_id) => {
                writeln!(formatter, "  old: {old_commit_id}")?;
            }
            ExistingSubmoduleGitlink::Conflicted => {
                writeln!(formatter, "  old: conflicted")?;
            }
        }
        writeln!(formatter, "  new: {target_commit_id}")?;
        write!(formatter, "Rewritten superproject commit: ")?;
        tx.write_commit_summary(formatter.as_mut(), &new_commit)?;
        writeln!(formatter)?;
        if num_rebased > 0 {
            writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
        }
    }
    tx.finish(
        ui,
        format!("update git submodule {ui_path} to {target_commit_id}"),
    )
    .await?;
    Ok(())
}

enum ExistingSubmoduleGitlink {
    Normal(CommitId),
    Conflicted,
}

impl ExistingSubmoduleGitlink {
    fn as_normal(&self) -> Option<&CommitId> {
        match self {
            Self::Normal(commit_id) => Some(commit_id),
            Self::Conflicted => None,
        }
    }
}

fn inspect_existing_submodule_gitlink(
    value: &MergedTreeValue,
    ui_path: &str,
) -> Result<ExistingSubmoduleGitlink, CommandError> {
    match value.as_resolved() {
        Some(Some(TreeValue::GitSubmodule(commit_id))) => {
            return Ok(ExistingSubmoduleGitlink::Normal(commit_id.clone()));
        }
        Some(Some(_)) => return Err(user_error(format!("Path {ui_path} is not a Git submodule"))),
        Some(None) => {
            return Err(user_error(format!("Submodule path {ui_path} is absent"))
                .hinted("Only existing Git submodule gitlinks can be updated."));
        }
        None => {}
    }

    let mut has_submodule = false;
    let mut has_non_submodule = false;
    for tree_value in value.iter().flatten() {
        match tree_value {
            TreeValue::GitSubmodule(_) => has_submodule = true,
            TreeValue::File { .. } | TreeValue::Symlink(_) | TreeValue::Tree(_) => {
                has_non_submodule = true;
            }
        }
    }
    if has_submodule && !has_non_submodule {
        Ok(ExistingSubmoduleGitlink::Conflicted)
    } else {
        Err(user_error(format!(
            "Submodule path {ui_path} is not a Git submodule conflict"
        ))
        .hinted("Only conflicts involving Git submodule gitlinks can be resolved."))
    }
}

async fn resolve_submodule_revision(
    ui: &Ui,
    command: &CommandHelper,
    submodule_path: &Path,
    revision: &str,
    ui_path: &str,
) -> Result<jj_lib::backend::CommitId, CommandError> {
    if submodule_path.join(".jj").exists() {
        resolve_submodule_jj_revision(ui, command, submodule_path, revision).await
    } else {
        resolve_submodule_git_revision(submodule_path, revision, ui_path)
    }
}

async fn resolve_submodule_jj_revision(
    ui: &Ui,
    command: &CommandHelper,
    submodule_path: &Path,
    revision: &str,
) -> Result<jj_lib::backend::CommitId, CommandError> {
    let submodule_workspace_command = command
        .workspace_helper_at_head_no_snapshot(ui, submodule_path)
        .await?;
    let commit = submodule_workspace_command
        .resolve_single_rev(ui, &RevisionArg::from(revision.to_owned()))
        .await?;
    parse_git_submodule_commit_id(&commit.id().hex(), revision)
}

fn resolve_submodule_git_revision(
    submodule_path: &Path,
    revision: &str,
    ui_path: &str,
) -> Result<jj_lib::backend::CommitId, CommandError> {
    let git_repo = gix::open(submodule_path).map_err(|err| {
        user_error_with_message(format!("Failed to open Git submodule at {ui_path}"), err)
    })?;
    let id = git_repo
        .rev_parse_single(revision)
        .map_err(|err| revision_resolution_error("gix", revision, ui_path, err))?;
    let object = id
        .object()
        .map_err(|err| revision_resolution_error("gix", revision, ui_path, err))?;
    let commit = object
        .peel_to_commit()
        .map_err(|err| revision_resolution_error("gix", revision, ui_path, err))?;
    parse_git_submodule_commit_id(&commit.id.to_string(), revision)
}

fn parse_git_submodule_commit_id(
    hex: &str,
    revision: &str,
) -> Result<jj_lib::backend::CommitId, CommandError> {
    if hex.len() != 40 {
        return Err(user_error(format!(
            "Only SHA-1 Git submodule commits are supported, but {revision:?} resolved to {hex:?}"
        )));
    }
    jj_lib::backend::CommitId::try_from_hex(hex).ok_or_else(|| {
        user_error(format!(
            "Resolved invalid commit id {hex:?} for submodule revision {revision:?}"
        ))
    })
}

fn revision_resolution_error(
    tool: &str,
    revision: &str,
    ui_path: &str,
    err: impl std::fmt::Display,
) -> CommandError {
    user_error(format!(
        "Failed to resolve submodule revision {revision:?} at {ui_path} with {tool}"
    ))
    .hinted(err.to_string())
}
