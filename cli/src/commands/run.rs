// Copyright 2024 The Jujutsu Authors
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

//! This file contains the internal implementation of `run`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;

use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::fsmonitor::FsmonitorSettings;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::local_working_copy::EolConversionMode;
use jj_lib::local_working_copy::ExecChangeSetting;
use jj_lib::local_working_copy::TreeState;
use jj_lib::local_working_copy::TreeStateError;
use jj_lib::local_working_copy::TreeStateSettings;
use jj_lib::lock::FileLock;
use jj_lib::lock::FileLockError;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::tree::Tree;
use jj_lib::working_copy::SnapshotOptions;
use tokio::runtime::Builder;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinError;
use tokio::task::JoinSet;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::command_error::CommandErrorKind;
use crate::ui::Ui;

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("failed to checkout the commit {}", .0)]
    FailedCheckout(CommitId),
    #[error("the command '{}' failed with {} for commit {}", .0,.1, .2)]
    CommandFailure(String, ExitStatus, CommitId),
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error("failed to create path {} with {}", .0.to_string_lossy(), .1)]
    PathCreationFailure(PathBuf, io::Error),
    #[error("failed to load a commits tree")]
    TreeState(#[from] TreeStateError),
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    JobFailure(#[from] JoinError),
    #[error(transparent)]
    LockError(#[from] FileLockError),
}

impl From<RunError> for CommandError {
    fn from(value: RunError) -> Self {
        Self::new(CommandErrorKind::Cli, Box::new(value))
    }
}

/// Provision an isolated per-commit working copy under `base_path` and check
/// out `commit`'s tree into it. Returns the working-copy directory and its
/// initialized `TreeState`, ready for the caller to spawn a command in and
/// later snapshot.
async fn create_working_copy(
    base_path: &Path,
    commit: &Commit,
) -> Result<(PathBuf, TreeState), RunError> {
    // Per-commit working-copy directory, keyed by commit id so concurrent jobs
    // don't collide and so the working copy is reproducible across runs.
    let commit_path = base_path.join(commit.id().hex());
    // A previous `jj run` that failed before cleanup may have left this
    // directory behind. Clear it so the working_copy/state subdirs below
    // start from a clean slate.
    if commit_path.exists() {
        tracing::debug!(
            dir = ?commit_path,
            commit = commit.id().hex(),
            "removing leftover directory from a previous run"
        );
        fs::remove_dir_all(&commit_path)
            .map_err(|e| RunError::PathCreationFailure(commit_path.clone(), e))?;
    }
    tracing::debug!(
        dir = ?commit_path,
        commit = commit.id().hex(),
        "creating directory for commit"
    );
    fs::create_dir(&commit_path)
        .map_err(|e| RunError::PathCreationFailure(commit_path.clone(), e))?;

    tracing::debug!(?commit_path, "creating working copy paths for path");
    let working_copy_dir = commit_path.join("working_copy");
    let state_dir = commit_path.join("state");
    tracing::debug!(?working_copy_dir, ?state_dir, "creating paths for a commit");
    fs::create_dir(&working_copy_dir)
        .map_err(|e| RunError::PathCreationFailure(working_copy_dir.clone(), e))?;
    fs::create_dir(&state_dir).map_err(|e| RunError::PathCreationFailure(state_dir.clone(), e))?;
    let tree_state_settings = TreeStateSettings {
        conflict_marker_style: ConflictMarkerStyle::Snapshot,
        eol_conversion_mode: EolConversionMode::None,
        exec_change_setting: ExecChangeSetting::Auto,
        fsmonitor_settings: FsmonitorSettings::None,
    };
    let mut tree_state = TreeState::init(
        commit.store().clone(),
        working_copy_dir.clone(),
        state_dir,
        &tree_state_settings,
    )?;
    tree_state
        .check_out(&commit.tree())
        .map_err(|_| RunError::FailedCheckout(commit.id().clone()))?;

    Ok((working_copy_dir, tree_state))
}

/// A command and its arguments, as parsed from the command line.
struct CommandSpec {
    program: String,
    args: Vec<String>,
}

impl fmt::Display for CommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.program)?;
        for arg in &self.args {
            f.write_str(" ")?;
            f.write_str(arg)?;
        }
        Ok(())
    }
}

/// The result of a single command invocation.
struct RunJob {
    /// The old `CommitId` of the commit.
    old_id: CommitId,
    /// The new tree generated from the commit. `None` when the command wasn't
    /// run (i.e. the commit was skipped).
    new_tree: Option<Tree>,
    /// Was the tree even modified.
    dirty: bool,
    /// Bytes the subprocess wrote to its stdout, captured in full.
    stdout: Vec<u8>,
    /// Bytes the subprocess wrote to its stderr, captured in full.
    stderr: Vec<u8>,
}

// TODO: make this more revset/commit stream friendly.
async fn run_inner(
    tx: &WorkspaceCommandTransaction<'_>,
    sender: Sender<RunJob>,
    handle: &tokio::runtime::Handle,
    spec: Arc<CommandSpec>,
    base_path: Arc<PathBuf>,
    commits: Arc<Vec<Commit>>,
    jobs: usize,
) -> Result<(), RunError> {
    let base_ignores = tx.base_workspace_helper().base_ignores().unwrap().clone();
    let semaphore = Arc::new(Semaphore::new(jobs));
    let mut command_futures: JoinSet<Result<RunJob, RunError>> = JoinSet::new();
    for commit in commits.iter() {
        let permit_future = semaphore.clone().acquire_owned();
        let base_ignores = base_ignores.clone();
        let base_path = base_path.clone();
        let commit = commit.clone();
        let spec = spec.clone();
        command_futures.spawn_on(
            async move {
                let _permit: OwnedSemaphorePermit =
                    permit_future.await.expect("semaphore not closed");
                // TODO: handle/propagate error here
                rewrite_commit(base_ignores, base_path, commit, spec).await
            },
            handle,
        );
    }

    while let Some(res) = command_futures.join_next().await {
        let done = match res {
            Ok(rj) => rj?,
            Err(err) => return Err(RunError::JobFailure(err)),
        };
        let should_quit = sender.send(done).await.is_err();
        if should_quit {
            tracing::debug!(
                ?should_quit,
                "receiver is no longer available, exiting loop"
            );
            break;
        }
    }
    Ok(())
}

/// Run `spec` against `commit`. The caller is responsible for committing any
/// returned new tree to the repo.
///
/// Each invocation provisions its own per-commit working copy under
/// `base_path` so multiple `rewrite_commit` futures can do their work in
/// parallel without contending on shared state.
async fn rewrite_commit(
    base_ignores: Arc<GitIgnoreFile>,
    base_path: Arc<PathBuf>,
    commit: Commit,
    spec: Arc<CommandSpec>,
) -> Result<RunJob, RunError> {
    let old_id = commit.id().clone();

    let (working_copy_dir, mut tree_state) = create_working_copy(&base_path, &commit).await?;

    // TODO: Later this should take some trait which allows `run` to integrate with
    // something like Bazels RE protocol.
    // e.g
    // ```
    // let mut executor /* Arc<dyn CommandExecutor> */ = store.get_executor();
    // let command = executor.spawn(...)?; // RE or separate processes depending on impl.
    // ...
    // ```
    tracing::debug!("trying to run command '{}' on commit {}", spec, commit.id());
    // Pipe and buffer the subprocess's stdout/stderr so we can emit them
    // atomically to the parent's stdout/stderr after the process exits. Writing
    // concurrently from multiple jobs would interleave output.
    let command = tokio::process::Command::new(&spec.program)
        .args(&spec.args)
        // set cwd to the subdirectory inside the working copy.
        .current_dir(&working_copy_dir)
        .env("JJ_WORKSPACE_ROOT", &working_copy_dir)
        .env("JJ_CHANGE_ID", commit.change_id().reverse_hex())
        .env("JJ_COMMIT_ID", commit.id().hex())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // No zombies allowed.
        .spawn()?;

    let output = command.wait_with_output().await?;

    // TODO: Handle error here
    if !output.status.success() {
        return Err(RunError::CommandFailure(
            spec.to_string(),
            output.status,
            old_id.clone(),
        ));
    }

    let options = SnapshotOptions {
        base_ignores,
        // TODO: read from current wc/settings
        start_tracking_matcher: &EverythingMatcher,
        progress: None,
        // TODO: read from current wc/settings
        max_new_file_size: 64_000_u64, // 64 MB for now,
        force_tracking_matcher: &NothingMatcher,
    };
    tracing::debug!("trying to snapshot the new tree");
    let (dirty, _) = tree_state.snapshot(&options).await.unwrap();
    if !dirty {
        tracing::debug!(
            "commit {} was not modified as the passed command did not modify any tracked files",
            commit.id()
        );
    }

    let rewritten_id = tree_state.current_tree().tree_ids();
    let new_id = rewritten_id.as_resolved().unwrap();

    let new_tree = commit.store().get_tree(RepoPathBuf::root(), new_id).await?;

    // TODO: Serialize the new tree into /output/{id-tree} for a cache lookup
    // TODO: supersede with a custom workspace implementation

    Ok(RunJob {
        old_id,
        new_tree: Some(new_tree),
        dirty,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// Run a command across a set of revisions.
///
/// Checks out each revision in an isolated working copy, runs the command, then
/// amends the revision with the resulting working copy. Descendants are rebased
/// on top of the amended revisions.
///
/// The command is executed from the root of the repository with the following
/// environment variables set:
///
/// - JJ_CHANGE_ID
/// - JJ_COMMIT_ID
/// - JJ_WORKSPACE_ROOT
///
/// # Example
///
/// ```shell
/// # Run pre-commit on your local work
/// $ jj run -j 4 -- pre-commit run .github/pre-commit.yaml
/// ```
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub struct RunArgs {
    /// Command to run across all selected revisions
    #[arg(value_name = "COMMAND")]
    command: String,

    /// Arguments to pass to the command
    ///
    /// Hint: Use a `--` separator to allow passing arguments starting with `-`.
    /// For example `jj run --revisions=... -- cargo build --release`.
    #[arg(value_name = "ARGS")]
    args: Vec<String>,

    /// The revisions to change
    #[arg(
        long = "revision",
        short,
        default_value = "reachable(@, mutable())",
        value_name = "REVSETS",
        alias = "revisions"
    )]
    revisions: Vec<RevisionArg>,

    /// A no-op option to match the interface of `git rebase -x`
    #[arg(short = 'x', hide = true)]
    exec: bool,

    /// How many processes should run in parallel
    #[arg(long, short, default_value_t = 1)]
    jobs: usize,
}

pub async fn cmd_run(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RunArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    // The commits are already returned in reverse topological order.
    let resolved_commits: Vec<_> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()
        .await?;

    workspace_command
        .check_rewritable(resolved_commits.iter().ids())
        .await?;

    tracing::debug!(?args.jobs, "starting with `jj run` with available threads");

    let rt = {
        let mut builder = Builder::new_multi_thread();
        builder.enable_io();
        builder.build().unwrap()
    };
    // TODO: Add a extension point for custom output/status aggregation.
    let mut done_commits = HashSet::new();
    let (sender_tx, mut receiver) = mpsc::channel(args.jobs);

    let store = workspace_command.repo().store().clone();
    let mut tx = workspace_command.start_transaction();
    let repo_path = tx.base_workspace_helper().repo_path();

    // Per-commit working copies are now created on demand inside
    // `rewrite_commit`; we just need the parent directory to exist.
    // The lock is held for the duration of the run, including the cleanup that
    // removes the run directory, and released on drop.
    // TODO: should be stored in a backend and not hardcoded.
    // The parent() call is needed to not write under `.jj/repo/`.
    let base_path = repo_path.parent().unwrap().join("run").join("default");
    if !base_path.exists() {
        tracing::debug!(?base_path, "does not exist, so creating it");
        fs::create_dir_all(&base_path)?;
    }
    // Keep the lock file *beside* `base_path` (e.g. `run/default.lock`) rather
    // than inside it. The directory is removed during cleanup while the lock is
    // still held, and on Windows a directory can't be removed while it contains
    // an open file handle. A sibling lock file avoids that and lets us hold the
    // lock across the deletion, so no other process can race in between.
    let lock = FileLock::lock(base_path.with_extension("lock")).map_err(RunError::from)?;
    let base_path = Arc::new(base_path);
    let stored_len = resolved_commits.len();

    let spec = Arc::new(CommandSpec {
        program: args.command.clone(),
        args: args.args.clone(),
    });
    let mut rewritten_commits = HashMap::new();

    // Drive the producer (run_inner) and consumer (receive loop) concurrently
    // so that each subprocess's output is emitted as soon as it finishes rather
    // than after all subprocesses complete.
    let ((), visited) = futures::try_join!(
        async {
            run_inner(
                &tx,
                sender_tx,
                rt.handle(),
                spec.clone(),
                base_path,
                Arc::new(resolved_commits.clone()),
                args.jobs,
            )
            .await
            .map_err(CommandError::from)
        },
        async {
            let mut visited = 0;
            while let Some(res) = receiver.recv().await {
                // Emit the subprocess's captured streams. Acquiring
                // `ui.stdout()` / `ui.stderr()` for the duration of the
                // write keeps each commit's output from interleaving with
                // another's.
                if !res.stdout.is_empty() {
                    let mut out = ui.stdout();
                    out.write_all(&res.stdout)?;
                }
                if !res.stderr.is_empty() {
                    let mut err = ui.stderr();
                    err.write_all(&res.stderr)?;
                }
                if res.dirty
                    && let Some(new_tree) = res.new_tree
                {
                    done_commits.insert(res.old_id.clone());
                    rewritten_commits.insert(res.old_id.clone(), new_tree);
                }
                visited += 1;
                if visited == stored_len {
                    break;
                }
            }
            Ok::<_, CommandError>(visited)
        },
    )?;

    let run_path = repo_path.parent().unwrap().join("run").join("default");
    // The operation was a no-op, bail.
    if rewritten_commits.is_empty() {
        // Yeet everything, caching is better implemented in a follow-up.
        fs::remove_dir_all(&run_path)?;

        writeln!(
            ui.stderr(),
            "No commits were rewritten as the command did not modify any tracked files"
        )?;
        tx.finish(ui, format!("run: No-op on {visited} commits with {spec}"))
            .await?;
        return Ok(());
    }

    // The command did something, so rewrite the commits.
    let mut count: u32 = 0;
    // TODO: handle the `--reparent` case here.
    tx.repo_mut()
        .transform_descendants(
            resolved_commits.iter().ids().cloned().collect_vec(),
            async |rewriter| {
                let old_id = rewriter.old_commit().id().clone();
                let builder = rewriter.rebase().await?;
                // Only rewrite the tree if the command changed it. Descendants
                // that weren't part of the input set still need to be rebased
                // but keep their original tree.
                if let Some(new_tree) = rewritten_commits.get(&old_id) {
                    let new_tree_id = new_tree.id().clone();
                    count += 1;
                    builder
                        .set_tree(MergedTree::resolved(store.clone(), new_tree_id))
                        .write()
                        .await?;
                } else {
                    builder.write().await?;
                }
                Ok(())
            },
        )
        .await?;
    writeln!(ui.stderr(), "Rewrote {count} commits with {spec}")?;

    // Yeet everything, caching is implemented in a follow-up.
    fs::remove_dir_all(&run_path)?;

    tx.finish(ui, format!("run: rewrite {count} commits with {spec}"))
        .await?;

    drop(lock);
    Ok(())
}
