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

use std::cmp::min;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
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
use jj_lib::matchers::Matcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::tree::Tree;
use jj_lib::working_copy::SnapshotOptions;
use tokio::runtime::Builder;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinError;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::command_error::CommandErrorKind;
use crate::ui::Ui;

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("failed to checkout the commit {}", .0)]
    FailedCheckout(CommitId),
    #[error("the command '{}' failed with {}", .0, .1)]
    CommandFailure(String, ExitStatus),
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error("failed to create path {}: {}", .0.to_string_lossy(), .1)]
    PathCreationFailure(PathBuf, io::Error),
    #[error("failed to delete path {}: {}", .0.to_string_lossy(), .1)]
    PathDeletionFailure(PathBuf, io::Error),
    #[error("failed to load a commit's tree")]
    TreeState(#[from] TreeStateError),
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    JobFailure(#[from] JoinError),
    #[error(transparent)]
    FileLock(#[from] FileLockError),
    #[error("invalid value for `run.jobs`: {0} (must be a positive integer)")]
    InvalidJobCount(i64),
}

impl From<RunError> for CommandError {
    fn from(value: RunError) -> Self {
        Self::new(CommandErrorKind::User, Box::new(value))
    }
}

fn default_tree_state_settings() -> TreeStateSettings {
    TreeStateSettings {
        conflict_marker_style: ConflictMarkerStyle::Snapshot,
        eol_conversion_mode: EolConversionMode::None,
        exec_change_setting: ExecChangeSetting::Auto,
        fsmonitor_settings: FsmonitorSettings::None,
    }
}

/// A workspace that's ready for a single job to run against.
///
/// Dropping the workspace releases its interprocess lock, freeing the slot for
/// the next worker.
struct RunWorkspace {
    working_copy_dir: PathBuf,
    tree_state: TreeState,
    /// Holds the slot's interprocess lock for the duration of the job.
    _lock: FileLock,
}

impl RunWorkspace {
    /// Persist the post-snapshot tree state to disk so the next slot
    /// acquisition can diff against it and only touch changed files.
    fn persist(&mut self) -> Result<(), RunError> {
        self.tree_state.save()?;
        Ok(())
    }
}

/// A command, its arguments, and the workspace-relative directory it should
/// run in.
struct CommandSpec {
    program: String,
    args: Vec<String>,
    /// Working directory for the subprocess, relative to the workspace root.
    /// Empty means run from the workspace root.
    subdir: Option<PathBuf>,
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

/// Manages a fixed-size pool of workspaces under `.jj/run/default/`.
///
/// Each workspace lives at `.jj/run/default/N/` with subdirs `working_copy/`
/// and `state/`. Its lockfile is the sibling `.jj/run/default/N.lock`.
/// Workspaces persist between `jj run` invocations so build artifacts can be
/// reused. Acquisition picks the first free workspace, so multiple concurrent
/// `jj run` processes cooperatively share the pool.
struct WorkspacePool {
    base_path: PathBuf,
    size: NonZeroUsize,
    /// Determines which untracked files left in a slot get pulled into the
    /// rewritten commit. Loaded once from `snapshot.auto-track`; essentially
    /// the user's `.gitignore` story for what counts as a build artifact.
    auto_tracking_matcher: Box<dyn Matcher>,
    /// When true, wipe each slot's working copy on acquisition so every commit
    /// starts from a freshly checked-out tree (no artifact reuse).
    clean: bool,
}

impl WorkspacePool {
    fn new(
        repo_path: &Path,
        size: NonZeroUsize,
        auto_tracking_matcher: Box<dyn Matcher>,
        clean: bool,
    ) -> Result<Self, RunError> {
        // The parent() call is needed to not write under `.jj/repo/`.
        let base_path = repo_path.parent().unwrap().join("run").join("default");
        fs::create_dir_all(&base_path)?;
        Ok(Self {
            base_path,
            size,
            auto_tracking_matcher,
            clean,
        })
    }

    async fn acquire(
        &self,
        commit: &Commit,
        base_ignores: Arc<GitIgnoreFile>,
    ) -> Result<RunWorkspace, RunError> {
        // Find a free slot. The first iteration may fail if another worker
        // (this process or another) holds every slot; sleep and retry.
        let mut cur_sleep = Duration::from_millis(10);
        let max_sleep = Duration::from_millis(250);
        let (slot_index, lock) = loop {
            if let Some(found) = self.try_acquire_any_slot()? {
                break found;
            }
            sleep(cur_sleep).await;
            cur_sleep = min(cur_sleep.saturating_mul(2), max_sleep);
        };

        let slot_path = self.slot_path(slot_index);
        let working_copy_dir = slot_path.join("working_copy");
        let state_dir = slot_path.join("state");
        let tree_state_path = state_dir.join("tree_state");

        let is_reused_workspace = tree_state_path.exists();
        let settings = default_tree_state_settings();
        let mut tree_state = if !self.clean && is_reused_workspace {
            // Load the persisted tree state so `check_out` below can diff
            // against it, only touching files that changed and removing files
            // no longer present in the new tree.
            //
            // Delete `tree_state` from disk before checkout to act as a dirty
            // marker. If we crash between here and the save at the end of the
            // job, the next acquisition will see the file absent and wipe the
            // slot rather than trusting inconsistent state.
            let ts = TreeState::load(
                commit.store().clone(),
                working_copy_dir.clone(),
                state_dir.clone(),
                &settings,
            )?;
            fs::remove_file(&tree_state_path)?;
            ts
        } else {
            // This is the first use of the workspace, the previous job crashed,
            // or --clean was passed. Wipe any leftover working copy so we start
            // from a clean slate, then use an in-memory empty tree state.
            // `tree_state` stays absent on disk until a successful job writes
            // it via `persist()`.
            fs::remove_file(&tree_state_path).or_else(|e| match e {
                e if e.kind() == io::ErrorKind::NotFound => Ok(()),
                e => Err(RunError::PathDeletionFailure(tree_state_path.clone(), e)),
            })?;
            fs::remove_dir_all(&working_copy_dir).or_else(|e| match e {
                e if e.kind() == io::ErrorKind::NotFound => Ok(()),
                e => Err(RunError::PathDeletionFailure(working_copy_dir.clone(), e)),
            })?;
            fs::remove_dir_all(&state_dir).or_else(|e| match e {
                e if e.kind() == io::ErrorKind::NotFound => Ok(()),
                e => Err(RunError::PathDeletionFailure(state_dir.clone(), e)),
            })?;
            fs::create_dir_all(&working_copy_dir)
                .map_err(|e| RunError::PathCreationFailure(working_copy_dir.clone(), e))?;
            fs::create_dir_all(&state_dir)
                .map_err(|e| RunError::PathCreationFailure(state_dir.clone(), e))?;
            TreeState::init_without_saving(
                commit.store().clone(),
                working_copy_dir.clone(),
                state_dir,
                &settings,
            )
        };

        tree_state
            .check_out(&commit.tree())
            .map_err(|_| RunError::FailedCheckout(commit.id().clone()))?;

        // If we checked out a revision with a completely empty tree,
        // TreeState::check_out() deletes the working_copy directory because it
        // recursively deletes empty directories until it reaches an error. In
        // normal workspaces, the .jj directory prevents the root directory from
        // being deleted, but our slots don't have that. It might be better to
        // make check_out explicitly avoid deleting the root directory, but that
        // needs to be a carefully considered change.
        fs::create_dir_all(&working_copy_dir)
            .map_err(|e| RunError::PathCreationFailure(working_copy_dir.clone(), e))?;

        // Remove files from a previous run that were ignored in the previous
        // revision but are not ignored in the new commit:
        // - Check out the new revsision
        // - Snapshot to discover any new files in the working copy
        // - Delete any new files
        // - Check out again to reset tree state
        if is_reused_workspace {
            let options = self.snapshot_options(base_ignores);
            tree_state.snapshot(&options).await.unwrap();
            let post_snapshot_tree = tree_state.current_tree().clone();
            let original_tree = commit.tree();
            let mut diff = original_tree.diff_stream(&post_snapshot_tree, &EverythingMatcher);
            let mut added_paths = Vec::new();
            while let Some(entry) = diff.next().await {
                let values = entry.values?;
                if values.before.is_absent() && values.after.is_present() {
                    added_paths.push(entry.path);
                }
            }
            drop(diff);
            for path in &added_paths {
                let abs = path.to_fs_path_unchecked(&working_copy_dir);
                if let Err(err) = fs::remove_file(&abs)
                    && err.kind() != io::ErrorKind::NotFound
                {
                    return Err(err.into());
                }
            }
            if !added_paths.is_empty() {
                tree_state
                    .check_out(&original_tree)
                    .map_err(|_| RunError::FailedCheckout(commit.id().clone()))?;
            }
        }

        Ok(RunWorkspace {
            working_copy_dir,
            tree_state,
            _lock: lock,
        })
    }

    fn snapshot_options(&self, base_ignores: Arc<GitIgnoreFile>) -> SnapshotOptions<'_> {
        SnapshotOptions {
            base_ignores,
            start_tracking_matcher: self.auto_tracking_matcher.as_ref(),
            progress: None,
            // TODO: read from current wc/settings
            max_new_file_size: 64_000_u64, // 64 MB for now
            force_tracking_matcher: &NothingMatcher,
        }
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.base_path.join(index.to_string())
    }

    fn slot_lock_path(&self, index: usize) -> PathBuf {
        self.base_path.join(format!("{index}.lock"))
    }

    /// Try to acquire any slot's lock without blocking. Returns the slot
    /// index and the held lock if one was available, `Ok(None)` if every
    /// slot was contended.
    fn try_acquire_any_slot(&self) -> Result<Option<(usize, FileLock)>, RunError> {
        for slot in 1..=self.size.get() {
            let slot_path = self.slot_path(slot);
            fs::create_dir_all(&slot_path)
                .map_err(|e| RunError::PathCreationFailure(slot_path.clone(), e))?;
            if let Some(lock) = FileLock::try_lock(self.slot_lock_path(slot))? {
                tracing::debug!(slot, "acquired pool slot");
                return Ok(Some((slot, lock)));
            }
        }
        Ok(None)
    }
}

/// The result of a single command invocation.
struct RunJob {
    /// The old `CommitId` of the commit.
    old_id: CommitId,
    /// The original tree of the commit before the command ran.
    old_tree: MergedTree,
    /// The new tree generated from the commit. `None` when the command wasn't
    /// run (i.e. the commit was skipped).
    new_tree: Option<Tree>,
    /// Was the tree even modified.
    dirty: bool,
    /// Bytes the subprocess wrote to its stdout, captured in full.
    stdout: Vec<u8>,
    /// Bytes the subprocess wrote to its stderr, captured in full.
    stderr: Vec<u8>,
    /// True if the command wasn't run because the per-commit working directory
    /// (the subdirectory `jj run` was invoked from) didn't exist in this
    /// commit's tree.
    skipped: bool,
    /// Exit status of the command, if it ran.
    status: Option<ExitStatus>,
}

// TODO: make this more revset/commit stream friendly.
#[allow(clippy::too_many_arguments)]
async fn run_inner(
    tx: &WorkspaceCommandTransaction<'_>,
    sender: Sender<RunJob>,
    handle: &tokio::runtime::Handle,
    spec: Arc<CommandSpec>,
    pool: Arc<WorkspacePool>,
    commits: Arc<Vec<Commit>>,
    jobs: usize,
    passthrough: bool,
) -> Result<(), RunError> {
    let base_ignores = tx.base_workspace_helper().base_ignores().unwrap().clone();
    let semaphore = Arc::new(Semaphore::new(jobs));
    let mut command_futures: JoinSet<Result<RunJob, RunError>> = JoinSet::new();
    for commit in commits.iter() {
        // Acquire the permit before spawning so tasks start in commit order.
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore not closed");
        let base_ignores = base_ignores.clone();
        let pool = pool.clone();
        let commit = commit.clone();
        let spec = spec.clone();
        command_futures.spawn_on(
            async move {
                let _permit = permit;
                // TODO: handle/propagate error here
                rewrite_commit(base_ignores, pool, commit, spec, passthrough).await
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
async fn rewrite_commit(
    base_ignores: Arc<GitIgnoreFile>,
    pool: Arc<WorkspacePool>,
    commit: Commit,
    spec: Arc<CommandSpec>,
    passthrough: bool,
) -> Result<RunJob, RunError> {
    let mut workspace = pool.acquire(&commit, base_ignores.clone()).await?;
    let working_copy_dir = workspace.working_copy_dir.clone();
    let old_id = commit.id().clone();
    let old_tree = commit.tree();

    // Resolve where the command should run. If the subdir doesn't exist in this
    // commit's checked-out tree, skip the commit entirely.
    let exec_dir = if let Some(subdir) = &spec.subdir {
        let exec_dir = working_copy_dir.join(subdir);
        if !exec_dir.is_dir() {
            tracing::debug!(
                ?exec_dir,
                commit = old_id.hex(),
                "subdirectory does not exist in commit; skipping"
            );
            // Persist the post-checkout state so the next pool acquisition
            // can diff from this tree even though no command ran.
            workspace.persist()?;
            return Ok(RunJob {
                old_id,
                old_tree,
                new_tree: None,
                dirty: false,
                stdout: Vec::new(),
                stderr: Vec::new(),
                skipped: true,
                status: None,
            });
        }
        exec_dir
    } else {
        working_copy_dir.clone()
    };

    // TODO: Later this should take some trait which allows `run` to integrate with
    // something like Bazels RE protocol.
    // e.g
    // ```
    // let mut executor /* Arc<dyn CommandExecutor> */ = store.get_executor();
    // let command = executor.spawn(...)?; // RE or separate processes depending on impl.
    // ...
    // ```
    tracing::debug!("trying to run command '{}' on commit {}", spec, commit.id());

    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&exec_dir)
        .env("JJ_WORKSPACE_ROOT", &working_copy_dir)
        .env("JJ_CHANGE_ID", commit.change_id().reverse_hex())
        .env("JJ_COMMIT_ID", commit.id().hex())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = if passthrough {
        // Connect stdout/stderr directly to the terminal so TTY-aware
        // programs work as expected. Capture is not possible; we wait
        // for the process to exit and return empty stdout/stderr.
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        let status = command.status().await?;
        Output {
            status,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    } else {
        // Pipe and buffer the subprocess's stdout/stderr so we can emit them
        // atomically to the parent's stdout/stderr after the process exits.
        // Writing concurrently from multiple jobs would interleave output.
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        command.spawn()?.wait_with_output().await?
    };

    let options = pool.snapshot_options(base_ignores);
    tracing::debug!("trying to snapshot the new tree");
    let (dirty, stats) = workspace.tree_state.snapshot(&options).await.unwrap();
    if !dirty {
        tracing::debug!(
            "commit {} was not modified as the passed command did not modify any tracked files",
            commit.id()
        );
    }

    if !output.status.success() {
        // Remove non-ignored untracked files left by the command. Ignored paths
        // are absent from `untracked_paths` and survive for build-artifact reuse.
        // This keeps the slot free of stale files that would cause silent
        // `skipped_files` collisions in the next `check_out`.
        for path in stats.untracked_paths.keys() {
            let abs = path.to_fs_path_unchecked(&working_copy_dir);
            if let Err(err) = fs::remove_file(&abs)
                && err.kind() != io::ErrorKind::NotFound
            {
                return Err(err.into());
            }
        }
    }

    // Persist the post-snapshot tree state so the next pool acquisition can
    // diff against it and only touch files that changed. Done unconditionally
    // so the slot is reusable even when the command failed.
    workspace.persist()?;

    let new_tree = if output.status.success() {
        let rewritten_id = workspace.tree_state.current_tree().tree_ids();
        let new_id = rewritten_id.as_resolved().unwrap();

        Some(commit.store().get_tree(RepoPathBuf::root(), new_id).await?)

        // TODO: Serialize the new tree into /output/{id-tree} for a cache
        // lookup TODO: supersede with a custom workspace implementation
    } else {
        None
    };

    Ok(RunJob {
        old_id,
        old_tree,
        new_tree,
        dirty,
        stdout: output.stdout,
        stderr: output.stderr,
        skipped: false,
        status: Some(output.status),
    })
}

/// Run a command across a set of revisions.
///
/// Checks out each revision in an isolated working copy, runs the command, then
/// amends the revision with the resulting working copy. By default, descendants
/// are rebased on top of the amended revisions, propagating the diff. Use
/// `--restore-descendants` to keep descendants' content unchanged instead.
///
/// The command is executed with the following environment variables set:
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
    #[arg(long = "revision", short, value_name = "REVSETS", alias = "revisions")]
    revisions: Vec<RevisionArg>,

    /// A no-op option to match the interface of `git rebase -x`
    #[arg(short = 'x', hide = true)]
    exec: bool,

    /// How many processes should run in parallel
    ///
    /// Overrides the `run.jobs` config setting. Defaults to 1 if neither is
    /// set.
    #[arg(long, short)]
    jobs: Option<usize>,

    /// Run the command from the working-copy root in each commit instead of
    /// from the subdirectory `jj run` was invoked from.
    #[arg(long)]
    root: bool,

    /// Delete each working copy before running the command
    ///
    /// By default `jj run` reuses working copies between invocations so build
    /// artifacts are preserved. With `--clean`, every commit starts from a
    /// freshly checked-out tree.
    #[arg(long)]
    clean: bool,

    /// Preserve the content (not the diff) when rebasing descendants
    #[arg(long)]
    restore_descendants: bool,

    /// Pass through stdout and stderr directly to the terminal
    ///
    /// The command's stdout and stderr are connected directly to the
    /// terminal (instead of being captured), so programs that behave
    /// differently on a TTY (e.g. colored output, progress bars) work
    /// as expected. Stdin is not inherited. Only one job is allowed
    /// since parallel jobs would interleave their output.
    #[arg(long)]
    passthrough: bool,
}

/// Precedence: `--jobs`, `run.jobs` config, 1.
fn resolve_jobs(
    workspace_command: &WorkspaceCommandHelper,
    jobs: Option<usize>,
) -> Result<NonZeroUsize, CommandError> {
    if let Some(j) = jobs {
        return NonZeroUsize::new(j).ok_or_else(|| {
            CommandError::new(
                CommandErrorKind::Cli,
                Box::new(RunError::InvalidJobCount(
                    i64::try_from(j).unwrap_or(i64::MAX),
                )),
            )
        });
    }
    if let Ok(size) = workspace_command.settings().get_int("run.jobs") {
        let size: usize = size
            .try_into()
            .map_err(|_| RunError::InvalidJobCount(size))?;
        return NonZeroUsize::new(size).ok_or_else(|| {
            CommandError::new(
                CommandErrorKind::Config,
                Box::new(RunError::InvalidJobCount(0)),
            )
        });
    }
    Ok(NonZeroUsize::MIN)
}

pub async fn cmd_run(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RunArgs,
) -> Result<(), CommandError> {
    let repo_path = command.workspace_loader()?.repo_path().to_path_buf();
    // TODO: should be stored in a backend and not hardcoded.
    let base_path = repo_path.parent().unwrap().join("run").join("default");
    fs::create_dir_all(&base_path)?;

    let mut workspace_command = command.workspace_helper(ui).await?;
    let mut resolved_commits: Vec<_> = if args.revisions.is_empty() {
        let revs = workspace_command.settings().get_string("revsets.run")?;
        workspace_command
            .parse_revset(ui, &RevisionArg::from(revs))?
            .evaluate_to_commits()?
            .try_collect()
            .await?
    } else {
        workspace_command
            .parse_union_revsets(ui, &args.revisions)?
            .evaluate_to_commits()?
            .try_collect()
            .await?
    };
    resolved_commits.reverse();

    workspace_command
        .check_rewritable(resolved_commits.iter().ids())
        .await?;

    let jobs = resolve_jobs(&workspace_command, args.jobs)?;

    if args.passthrough && jobs.get() > 1 {
        return Err(CommandError::new(
            CommandErrorKind::Cli,
            "cannot use --passthrough with more than one job".to_string(),
        ));
    }

    tracing::debug!(?jobs, "starting `jj run`");

    // Run each command from the subdirectory the user invoked `jj run` from,
    // unless `--root` overrides that. The subdir is relative to the workspace
    // root, which is canonical (per `CommandHelper::cwd` docs).
    let subdir = if args.root {
        None
    } else {
        Some(
            command
                .cwd()
                .strip_prefix(workspace_command.workspace_root())
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        )
    };

    let store = workspace_command.repo().store().clone();
    let auto_tracking_matcher = workspace_command.auto_tracking_matcher(ui)?;

    let mut tx = workspace_command.start_transaction();

    let rt = {
        let mut builder = Builder::new_multi_thread();
        builder.enable_io();
        builder.enable_time();
        builder.build().unwrap()
    };
    let mut done_commits = HashSet::new();
    let (sender_tx, mut receiver) = mpsc::channel(jobs.get());

    let pool = Arc::new(WorkspacePool::new(
        &repo_path,
        jobs,
        auto_tracking_matcher,
        args.clean,
    )?);

    let spec = Arc::new(CommandSpec {
        program: args.command.clone(),
        args: args.args.clone(),
        subdir,
    });
    let mut rewritten_commits = HashMap::new();

    // Drive the producer (run_inner) and consumer (receive loop) concurrently
    // so that each subprocess's output is emitted as soon as it finishes rather
    // than after all subprocesses complete.
    futures::try_join!(
        async {
            run_inner(
                &tx,
                sender_tx,
                rt.handle(),
                spec.clone(),
                pool.clone(),
                Arc::new(resolved_commits.clone()),
                jobs.get(),
                args.passthrough,
            )
            .await
            .map_err(CommandError::from)
        },
        async {
            while let Some(res) = receiver.recv().await {
                if res.skipped {
                    writeln!(
                        ui.stderr(),
                        "Skipped commit {}: directory does not exist: {}",
                        res.old_id.hex(),
                        spec.subdir.as_deref().unwrap_or(Path::new("")).display()
                    )?;
                } else {
                    if let Some(status) = res.status {
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
                        if !status.success() {
                            let commit = store.get_commit_async(&res.old_id).await?;
                            let mut error: CommandError =
                                RunError::CommandFailure(spec.to_string(), status).into();
                            error.add_formatted_hint_with(|formatter| {
                                write!(formatter, "Failed revision: ")?;
                                tx.write_commit_summary(formatter, &commit)
                            });
                            return Err(error);
                        }
                    }
                    if res.dirty
                        && let Some(new_tree) = res.new_tree
                    {
                        done_commits.insert(res.old_id.clone());
                        rewritten_commits.insert(res.old_id.clone(), (res.old_tree, new_tree));
                    }
                }
            }
            Ok::<_, CommandError>(())
        },
    )?;

    // The operation was a no-op, bail.
    if rewritten_commits.is_empty() {
        tx.finish(ui, "run: nothing changed").await?;
        return Ok(());
    }

    // The command did something, so rewrite the commits.
    let restore_descendants = args.restore_descendants;
    let mut count: u32 = 0;
    let mut num_reparented: u32 = 0;
    tx.repo_mut()
        .transform_descendants(
            resolved_commits.iter().ids().cloned().collect_vec(),
            async |rewriter| {
                let old_id = rewriter.old_commit().id().clone();
                match (rewritten_commits.get(&old_id), restore_descendants) {
                    (Some((_, new_tree)), true) => {
                        let builder = rewriter.rebase().await?;
                        count += 1;
                        // Use the command result on top of the commit's
                        // original tree, ignoring rewrites of its ancestors.
                        builder
                            .set_tree(MergedTree::resolved(store.clone(), new_tree.id().clone()))
                            .write()
                            .await?;
                    }
                    (Some((old_tree, new_tree)), false) => {
                        let builder = rewriter.rebase().await?;
                        count += 1;
                        // Apply the diff the command introduced (new_tree -
                        // old_tree) on top of the rebased tree, propagating
                        // ancestor rewrites via the normal rebase merge.
                        let rebased_tree = builder.tree();
                        let merged = MergedTree::merge(Merge::from_vec(vec![
                            (
                                MergedTree::resolved(store.clone(), new_tree.id().clone()),
                                "command result".to_owned(),
                            ),
                            (old_tree.clone(), "original commit".to_owned()),
                            (rebased_tree, "rebased".to_owned()),
                        ]))
                        .await?;
                        builder.set_tree(merged).write().await?;
                    }
                    (None, true) => {
                        // Descendant outside the run set — keep its content.
                        rewriter.reparent().write().await?;
                        num_reparented += 1;
                    }
                    (None, false) => {
                        // Default: propagate the diff into descendants.
                        rewriter.rebase().await?.write().await?;
                    }
                }
                Ok(())
            },
        )
        .await?;
    writeln!(ui.stderr(), "Rewrote {count} commits.")?;
    if restore_descendants && num_reparented > 0 {
        writeln!(
            ui.stderr(),
            "Rebased {num_reparented} descendant commits (while preserving their content)."
        )?;
    }
    tx.finish(ui, format!("run: rewrite {count} commits"))
        .await?;

    Ok(())
}
