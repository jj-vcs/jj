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

use futures::future::try_join_all;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::commit_builder::DetachedCommitBuilder;
use jj_lib::file_util;
use jj_lib::file_util::IoResultExt as _;
#[cfg(feature = "git")]
use jj_lib::git::GitSubprocessOptions;
#[cfg(feature = "git")]
use jj_lib::git::create_worktree;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::workspace::Workspace;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::command_error::internal_error_with_message;
use crate::command_error::user_error;
#[cfg(feature = "git")]
use crate::commands::git::maybe_add_gitignore;
use crate::description_util::add_trailers;
use crate::description_util::join_message_paragraphs;
use crate::ui::Ui;

/// How to handle sparse patterns when creating a new workspace.
#[derive(clap::ValueEnum, Clone, Debug, Eq, PartialEq)]
pub(crate) enum SparseInheritance {
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

    /// Adopt the files already in the destination directory
    ///
    /// The destination directory must already exist and must not contain a
    /// `.jj` directory. Files that differ from the new working-copy commit are
    /// reset to match it, so the result is the same as without `--adopt`. The
    /// previous contents can be recovered with `jj undo`.
    ///
    /// This is useful in combination with a copy-on-write filesystem, such as
    /// Btrfs or ZFS, to create a workspace without writing all its files.
    ///
    /// Example:
    /// ```shell
    /// btrfs subvolume snapshot src dst
    /// rm -rf dst/.jj dst/.git
    /// jj -R src workspace add --adopt dst
    /// ```
    #[arg(long, verbatim_doc_comment)]
    adopt: bool,

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
    #[arg(long = "revision", short, value_name = "REVSETS", alias = "revisions")]
    revisions: Vec<RevisionArg>,

    /// The change description to use
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,

    /// Create a corresponding Git worktree for this workspace
    ///
    /// By default, a Git worktree is created when the current workspace is
    /// colocated and the [git.colocate config] is `true`.
    ///
    /// [git.colocate config]:
    ///     https://docs.jj-vcs.dev/latest/config/#default-colocation
    #[cfg(feature = "git")]
    #[arg(long, conflicts_with = "no_colocate")]
    colocate: bool,

    /// Do not create a Git worktree for this workspace
    #[cfg(feature = "git")]
    #[arg(long, conflicts_with = "colocate")]
    no_colocate: bool,

    /// How to handle sparse patterns when creating a new workspace.
    #[arg(long, value_enum, default_value_t = SparseInheritance::Copy)]
    sparse_patterns: SparseInheritance,
}

#[instrument(skip_all)]
pub async fn cmd_workspace_add(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &WorkspaceAddArgs,
) -> Result<(), CommandError> {
    check_new_working_copy_writable(command)?;
    let mut old_workspace_command = command.workspace_helper(ui).await?;
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

    let repo = old_workspace_command.repo().clone();
    if repo.view().get_wc_commit_id(&workspace_name).is_some() {
        return Err(user_error(format!(
            "Workspace named '{name}' already exists",
            name = workspace_name.as_symbol()
        )));
    }
    // Resolve the parents before creating the workspace so an invalid revset
    // doesn't leave a half-created workspace behind. If no parent revisions are
    // specified, create a working-copy commit based on the parents of the current
    // working-copy commit.
    let parents = if args.revisions.is_empty() {
        // Check out parents of the current workspace's working-copy commit, or the
        // root if there is no working-copy commit in the current workspace.
        if let Some(old_wc_commit) = old_workspace_command.get_wc_commit().await? {
            old_wc_commit.parents().await?
        } else {
            vec![repo.store().root_commit()]
        }
    } else {
        try_join_all(
            old_workspace_command
                .resolve_some_revsets(ui, &args.revisions)
                .await?
                .iter()
                .map(|id| repo.store().get_commit_async(id)),
        )
        .await?
    };

    if args.adopt {
        if !destination_path.is_dir() {
            return Err(user_error("Destination path is not a directory"));
        }
        if destination_path.join(".jj").exists() {
            return Err(
                user_error("Destination path already contains a .jj directory").hinted(
                    "If this directory was copied from an existing workspace, delete its .jj \
                     directory before adopting it.",
                ),
            );
        }
    } else if !destination_path.exists() {
        fs::create_dir(&destination_path).context(&destination_path)?;
    } else if !file_util::is_empty_dir(&destination_path)? {
        return Err(user_error(
            "Destination path exists and is not an empty directory",
        ));
    }

    #[cfg(feature = "git")]
    {
        let should_colocate = if args.colocate {
            true
        } else if args.no_colocate {
            false
        } else {
            old_workspace_command.working_copy_shared_with_git()
                && old_workspace_command.settings().get_bool("git.colocate")?
        };
        if should_colocate {
            let subprocess_options =
                GitSubprocessOptions::from_settings(old_workspace_command.settings())?;
            create_worktree(repo.store(), subprocess_options, &destination_path)?;
            writeln!(ui.status(), "Created Git worktree for the new workspace.")?;
        }
    }

    // When adopting, the working-copy commit needs to exist before the
    // workspace so the working copy can be initialized to it without touching
    // the files.
    let adopted_wc_commit = if args.adopt {
        let mut tx = old_workspace_command.start_transaction();
        let commit_builder =
            new_wc_commit_builder(ui, &mut tx, &parents, &args.message_paragraphs).await?;
        Some(commit_builder.write_hidden().await?)
    } else {
        None
    };

    let working_copy_factory = command.get_working_copy_factory()?;
    let repo_path = old_workspace_command.repo_path();
    // If we add per-workspace configuration, we'll need to reload settings for
    // the new workspace.
    let (new_workspace, repo) = if let Some(wc_commit) = &adopted_wc_commit {
        Workspace::adopt_workspace_with_existing_repo(
            &destination_path,
            repo_path,
            &repo,
            working_copy_factory,
            workspace_name.clone(),
            Some(old_workspace_command.workspace_name()),
            wc_commit,
        )
        .await?
    } else {
        Workspace::init_workspace_with_existing_repo(
            &destination_path,
            repo_path,
            &repo,
            working_copy_factory,
            workspace_name.clone(),
            Some(old_workspace_command.workspace_name()),
        )
        .await?
    };
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
    // Keep the colocated Git worktree from tracking Jujutsu's own files.
    #[cfg(feature = "git")]
    maybe_add_gitignore(&new_workspace_command)?;

    if args.adopt {
        // Record the adopted files so that any differences from the
        // working-copy commit can be recovered after resetting them below.
        new_workspace_command.maybe_snapshot(ui).await?;
    }

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
        let (mut locked_ws, _wc_commit) =
            new_workspace_command.start_working_copy_mutation().await?;
        locked_ws
            .locked_wc()
            .set_sparse_patterns(sparse_patterns)
            .await
            .map_err(|err| internal_error_with_message("Failed to set sparse patterns", err))?;
        let operation_id = locked_ws.locked_wc().old_operation_id().clone();
        locked_ws.finish(operation_id).await?;
    }

    if let Some(adopted_wc_commit) = adopted_wc_commit {
        let wc_commit = new_workspace_command.get_wc_commit().await?.unwrap();
        if wc_commit.id() != adopted_wc_commit.id() {
            let mut tx = new_workspace_command.start_transaction();
            tx.repo_mut()
                .rewrite_commit(&wc_commit)
                .set_tree(adopted_wc_commit.tree())
                .write()
                .await?;
            tx.finish(
                ui,
                format!(
                    "reset adopted files in workspace {name}",
                    name = workspace_name.as_symbol()
                ),
            )
            .await?;
            writeln!(
                ui.warning_default(),
                r#"Reset files in "{}" to match {}. To recover the previous contents, run `jj undo`."#,
                args.destination,
                if args.revisions.is_empty() {
                    format!(
                        "@- in the source workspace '{}'",
                        old_workspace_command.workspace_name().as_symbol()
                    )
                } else if parents.len() > 1 {
                    "the specified parent commits".to_string()
                } else {
                    "the specified parent commit".to_string()
                }
            )?;
        }
    } else {
        let mut tx = new_workspace_command.start_transaction();
        // Attribute the operation to the workspace the command was run in, so e.g.
        // `jj undo` there undoes it
        tx.set_workspace_name(old_workspace_command.workspace_name());
        let commit_builder =
            new_wc_commit_builder(ui, &mut tx, &parents, &args.message_paragraphs).await?;
        let new_wc_commit = commit_builder.write(tx.repo_mut()).await?;
        tx.edit(&new_wc_commit)?;
        tx.finish(
            ui,
            format!(
                "create initial working-copy commit in workspace {name}",
                name = workspace_name.as_symbol()
            ),
        )
        .await?;
    }
    Ok(())
}

async fn new_wc_commit_builder(
    ui: &Ui,
    tx: &mut WorkspaceCommandTransaction<'_>,
    parents: &[Commit],
    message_paragraphs: &[String],
) -> Result<DetachedCommitBuilder, CommandError> {
    let tree = merge_commit_trees(tx.repo(), parents).await?;
    let parent_ids = parents.iter().ids().cloned().collect_vec();
    let mut commit_builder = tx.repo_mut().new_commit(parent_ids, tree).detach();
    let mut description = join_message_paragraphs(message_paragraphs);
    if !description.is_empty() {
        // The first trailer would become the first line of the description.
        // Also, a commit with no description is treated in a special way in jujutsu: it
        // can be discarded as soon as it's no longer the working copy. Adding a
        // trailer to an empty description would break that logic.
        commit_builder.set_description(description);
        description = add_trailers(ui, tx, &commit_builder).await?;
    }
    commit_builder.set_description(&description);
    Ok(commit_builder)
}

// Like check_working_copy_writable(), but --at-op is respected.
fn check_new_working_copy_writable(command: &CommandHelper) -> Result<(), CommandError> {
    let hint = if command.global_args().ignore_working_copy {
        "Don't use --ignore-working-copy."
    } else if command.global_args().no_integrate_operation {
        "Don't use --no-integrate-operation."
    } else {
        return Ok(());
    };
    Err(user_error("This command must be able to update the working copy.").hinted(hint))
}
