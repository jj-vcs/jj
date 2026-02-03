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

use std::fs;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::file_util;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::git;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::workspace::Workspace;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::internal_error_with_message;
use crate::command_error::user_error;
use crate::description_util::add_trailers;
use crate::description_util::join_message_paragraphs;
use crate::ui::Ui;

/// How to handle sparse patterns when creating a new workspace.
#[derive(clap::ValueEnum, Clone, Debug, Eq, PartialEq)]
enum SparseInheritance {
    /// Copy all sparse patterns from the current workspace.
    Copy,
    /// Include all files in the new workspace.
    Full,
    /// Clear all files from the workspace (it will be empty).
    Empty,
}

/// Add a workspace
///
/// By default, the new workspace inherits the sparse patterns of the current
/// workspace. You can override this with the `--sparse-patterns` option.
#[derive(clap::Args, Clone, Debug)]
pub struct WorkspaceAddArgs {
    /// Where to create the new workspace
    #[arg(value_hint = clap::ValueHint::DirPath)]
    destination: String,

    /// A name for the workspace
    ///
    /// To override the default, which is the basename of the destination
    /// directory.
    #[arg(long)]
    name: Option<WorkspaceNameBuf>,

    /// A list of parent revisions for the working-copy commit of the newly
    /// created workspace. You may specify nothing, or any number of parents.
    ///
    /// If no revisions are specified, the new workspace will be created, and
    /// its working-copy commit will exist on top of the parent(s) of the
    /// working-copy commit in the current workspace, i.e. they will share the
    /// same parent(s).
    ///
    /// If any revisions are specified, the new workspace will be created, and
    /// the new working-copy commit will be created with all these revisions as
    /// parents, i.e. the working-copy commit will exist as if you had run `jj
    /// new r1 r2 r3 ...`.
    #[arg(long, short, value_name = "REVSETS")]
    revision: Vec<RevisionArg>,

    /// The change description to use
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,

    /// How to handle sparse patterns when creating a new workspace.
    #[arg(long, value_enum, default_value_t = SparseInheritance::Copy)]
    sparse_patterns: SparseInheritance,
}

#[instrument(skip_all)]
pub fn cmd_workspace_add(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceAddArgs,
) -> Result<(), CommandError> {
    let old_workspace_command = command.workspace_helper(ui)?;
    let destination_path = command.cwd().join(&args.destination);
    let workspace_name = if let Some(name) = &args.name {
        name.to_owned()
    } else {
        let file_name = destination_path.file_name().unwrap();
        file_name
            .to_str()
            .ok_or_else(|| user_error("Destination path is not valid UTF-8"))?
            .into()
    };
    if workspace_name.as_str().is_empty() {
        return Err(user_error("New workspace name cannot be empty"));
    }

    let repo = old_workspace_command.repo();
    if repo.view().get_wc_commit_id(&workspace_name).is_some() {
        return Err(user_error(format!(
            "Workspace named '{name}' already exists",
            name = workspace_name.as_symbol()
        )));
    }
    if !destination_path.exists() {
        fs::create_dir(&destination_path).context(&destination_path)?;
    } else if !file_util::is_empty_dir(&destination_path)? {
        return Err(user_error(
            "Destination path exists and is not an empty directory",
        ));
    }

    let parent_commit_ids: Vec<CommitId> = if args.revision.is_empty() {
        if let Some(old_wc_commit_id) = repo
            .view()
            .get_wc_commit_id(old_workspace_command.workspace_name())
        {
            let old_wc_commit = repo.store().get_commit(old_wc_commit_id)?;
            old_wc_commit.parent_ids().to_vec()
        } else {
            vec![repo.store().root_commit_id().clone()]
        }
    } else {
        old_workspace_command
            .resolve_some_revsets(ui, &args.revision)?
            .into_iter()
            .collect()
    };

    // Best-effort Git worktree registration for colocated repos.
    //
    // Goal: minimally expose the new workspace to Git tooling (IDEs, hooks, etc.)
    // by writing Git worktree metadata.
    //
    // Limitation: JJ does not synchronize per-worktree Git HEAD/index with the
    // JJ working copy here, so Git commands that depend on worktree state may
    // be inaccurate. Failures must not block workspace creation.
    if crate::git_util::is_colocated_git_workspace(old_workspace_command.workspace(), repo)
        && let Some(parent_id) = parent_commit_ids
            .first()
            .filter(|id| *id != repo.store().root_commit_id())
    {
        let subprocess_options =
            git::GitSubprocessOptions::from_settings(old_workspace_command.settings())?;
        if let Err(err) = git::add_worktree(
            repo.as_ref(),
            &destination_path,
            parent_id,
            subprocess_options,
        ) {
            writeln!(
                ui.warning_default(),
                "Failed to create Git worktree for \"{}\": {err}",
                file_util::relative_path(command.cwd(), &destination_path).display(),
            )?;
        }
    }

    let working_copy_factory = command.get_working_copy_factory()?;
    let repo_path = old_workspace_command.repo_path();
    // If we add per-workspace configuration, we'll need to reload settings for
    // the new workspace.
    let (new_workspace, repo) = Workspace::init_workspace_with_existing_repo(
        &destination_path,
        repo_path,
        repo,
        working_copy_factory,
        workspace_name.clone(),
    )?;
    writeln!(
        ui.status(),
        "Created workspace in \"{}\"",
        file_util::relative_path(command.cwd(), &destination_path).display()
    )?;
    // Show a warning if the user passed a path without a separator, since they
    // may have intended the argument to only be the name for the workspace.
    if !args.destination.contains(std::path::is_separator) {
        writeln!(
            ui.warning_default(),
            r#"Workspace created inside current directory. If this was unintentional, delete the "{}" directory and run `jj workspace forget {name}` to remove it."#,
            args.destination,
            name = workspace_name.as_symbol()
        )?;
    }

    let mut new_workspace_command = command.for_workable_repo(ui, new_workspace, repo)?;

    let sparsity = match args.sparse_patterns {
        SparseInheritance::Full => None,
        SparseInheritance::Empty => Some(vec![]),
        SparseInheritance::Copy => {
            let sparse_patterns = old_workspace_command
                .working_copy()
                .sparse_patterns()?
                .to_vec();
            Some(sparse_patterns)
        }
    };

    if let Some(sparse_patterns) = sparsity {
        let (mut locked_ws, _wc_commit) = new_workspace_command.start_working_copy_mutation()?;
        locked_ws
            .locked_wc()
            .set_sparse_patterns(sparse_patterns)
            .block_on()
            .map_err(|err| internal_error_with_message("Failed to set sparse patterns", err))?;
        let operation_id = locked_ws.locked_wc().old_operation_id().clone();
        locked_ws.finish(operation_id)?;
    }

    let mut tx = new_workspace_command.start_transaction();

    // If no parent revisions are specified, create a working-copy commit based
    // on the parent of the current working-copy commit.
    let parents = parent_commit_ids
        .iter()
        .map(|id| tx.repo().store().get_commit(id))
        .collect::<Result<Vec<_>, _>>()?;

    let tree = merge_commit_trees(tx.repo(), &parents).block_on()?;
    let parent_ids = parents.iter().ids().cloned().collect_vec();
    let mut commit_builder = tx.repo_mut().new_commit(parent_ids, tree).detach();
    let mut description = join_message_paragraphs(&args.message_paragraphs);
    if !description.is_empty() {
        // The first trailer would become the first line of the description.
        // Also, a commit with no description is treated in a special way in jujutsu: it
        // can be discarded as soon as it's no longer the working copy. Adding a
        // trailer to an empty description would break that logic.
        commit_builder.set_description(description);
        description = add_trailers(ui, &tx, &commit_builder)?;
    }
    commit_builder.set_description(&description);
    let new_wc_commit = commit_builder.write(tx.repo_mut())?;

    tx.edit(&new_wc_commit)?;
    tx.finish(
        ui,
        format!(
            "create initial working-copy commit in workspace {name}",
            name = workspace_name.as_symbol()
        ),
    )?;
    Ok(())
}
