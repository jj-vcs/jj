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
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use itertools::Itertools;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::backend::MergedTreeId;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::fsmonitor::FsmonitorSettings;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::local_working_copy::TreeState;
use jj_lib::local_working_copy::TreeStateError;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::object_id::ObjectId;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::tree::Tree;
use jj_lib::working_copy::CheckoutOptions;
use jj_lib::working_copy::SnapshotOptions;
use pollster::FutureExt;
use tokio::runtime::Builder;
use tokio::sync::Mutex;
use tokio::task::JoinError;
use tokio::task::JoinSet;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::ui::Ui;

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error("failed to checkout the commit {}", .0)]
    FailedCheckout(CommitId),
    #[error("the command failed {} for {:?}", .0, .1)]
    CommandFailure(ExitStatus, CommitId),
    #[error(transparent)]
    IoError(#[from] io::Error),
    #[error("failed to create path {} with {:?}", .0.to_string_lossy(), .1)]
    PathCreationFailure(PathBuf, io::Error),
    #[error("failed to load a commits tree")]
    TreeState(#[from] TreeStateError),
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    JobFailure(#[from] JoinError),
}

impl From<RunError> for CommandError {
    fn from(value: RunError) -> Self {
        CommandError::new(crate::command_error::CommandErrorKind::Cli, Box::new(value))
    }
}

/// Creates the required directories for a StoredWorkingCopy.
/// Returns a tuple of (`output_dir`, `working_copy` and `state`).
fn create_working_copy_paths(path: &Path) -> Result<(PathBuf, PathBuf, PathBuf), RunError> {
    tracing::debug!(?path, "creating working copy paths for path");
    let output = path.join("output");
    let working_copy = path.join("working_copy");
    let state = path.join("state");
    tracing::debug!(
        ?output,
        ?working_copy,
        ?state,
        "creating paths for a commit"
    );
    std::fs::create_dir(&output).map_err(|e| RunError::PathCreationFailure(output.clone(), e))?;
    std::fs::create_dir(&working_copy)
        .map_err(|e| RunError::PathCreationFailure(working_copy.clone(), e))?;
    std::fs::create_dir(&state).map_err(|e| RunError::PathCreationFailure(state.clone(), e))?;

    Ok((output, working_copy, state))
}

/// Represent a `MergeTreeId` in a way that it may be used as a working-copy
/// name. This makes no stability guarantee, as the format may change at
/// any time.
fn to_wc_name(id: &MergedTreeId) -> String {
    match id {
        MergedTreeId::Legacy(tree_id) => tree_id.hex(),
        MergedTreeId::Merge(tree_ids) => {
            // Only obfuscate if we have multiple parents to a tree.
            let obfuscated = if tree_ids.num_sides() > 1 {
                let mut id_string = tree_ids
                    .map(|id| id.hex())
                    .iter_mut()
                    .enumerate()
                    .map(|(i, s)| {
                        if i % 2 != 0 {
                            s.push('-');
                        } else {
                            s.push('+');
                        }
                        s.to_owned()
                    })
                    .collect::<String>();
                // Remove the last character so we don't end on a `-` or `+`.
                id_string.pop();
                id_string
            } else {
                tree_ids.iter().map(|id| id.hex()).collect::<String>()
            };
            obfuscated
        }
    }
}

fn get_runtime(jobs: usize) -> tokio::runtime::Runtime {
    let mut builder = Builder::new_multi_thread();
    builder.max_blocking_threads(jobs);
    builder.enable_io();
    builder.build().unwrap()
}

/// A commit stored under `.jj/run/default/`
// TODO: Create a caching backend, which creates these on a dedicated thread or
// threadpool.
struct OnDiskCommit {
    /// The respective commit unmodified.
    commit: Commit,
    /// The output directory of the commit, contains stdout and stderr for it
    output_dir: PathBuf,
    /// Self-explanatory
    working_copy_dir: PathBuf,
    /// The commits `TreeState`, which is loaded on creation and then replaced
    /// if necessary. Protected by a Mutex for crossthread compatibility.
    tree_state: Mutex<TreeState>,
}

impl OnDiskCommit {
    fn new(
        commit: &Commit,
        output_dir: PathBuf,
        working_copy_dir: PathBuf,
        tree_state: Mutex<TreeState>,
    ) -> Self {
        Self {
            commit: commit.clone(),
            output_dir,
            working_copy_dir,
            tree_state,
        }
    }
}

fn create_output_files(id: &CommitId, path: &Path) -> Result<(File, File), RunError> {
    // We use the hex id of the commit here to allow multiple `std{in,err}`s to be
    // placed beside each other in a single output directory.
    tracing::debug!(?id, "creating output files (stdout, stderr) for commit");
    let stdout_path = path.join(format!("stdout.{}", id.hex()));
    let stderr_path = path.join(format!("stderr.{}", id.hex()));
    tracing::debug!(
        "trying to create paths (stdout) {} (stderr) {}",
        stdout_path.clone().display(),
        stderr_path.clone().display()
    );
    let stdout = File::create(&stdout_path)
        .map_err(|e| RunError::PathCreationFailure(stdout_path.clone(), e))?;
    let stderr = File::create(&stderr_path)
        .map_err(|e| RunError::PathCreationFailure(stderr_path.clone(), e))?;
    Ok((stdout, stderr))
}

fn create_working_copies(
    repo_path: &Path,
    commits: &[Commit],
) -> Result<Vec<Arc<OnDiskCommit>>, RunError> {
    let mut results = vec![];
    // TODO: should be stored in a backend and not hardcoded.
    // The parent() call is needed to not write under `.jj/repo/`.
    let base_path = repo_path.parent().unwrap().join("run").join("default");
    if !base_path.exists() {
        tracing::debug!(?base_path, "does not exist, so creating it");
        fs::create_dir_all(&base_path)?;
    }
    tracing::debug!(path = ?base_path, "creating working copies in path: ");
    for commit in commits {
        let name = to_wc_name(commit.tree_id());
        let commit_path = base_path.join(name.as_str());
        if !commit_path.exists() {
            tracing::debug!(
                dir = ?commit_path,
                commit = commit.id().hex(),
                "creating directory for commit"
            );
            fs::create_dir(&commit_path)
                .map_err(|e| RunError::PathCreationFailure(commit_path.clone(), e))?;
        }
        tracing::debug!("skipped directory creation, as it already exists");

        let (output_dir, working_copy_dir, state_dir) = create_working_copy_paths(&commit_path)?;
        let tree_state = {
            tracing::debug!(
                commit = commit.id().hex(),
                "trying to create a treestate for commit"
            );
            let mut tree_state = TreeState::init(
                commit.store().clone(),
                working_copy_dir.clone(),
                state_dir.clone(),
            )?;
            tree_state
                .check_out(
                    &commit.tree()?,
                    &CheckoutOptions {
                        conflict_marker_style: ConflictMarkerStyle::Diff,
                    },
                )
                .map_err(|_| RunError::FailedCheckout(commit.id().clone()))?;
            Mutex::new(tree_state)
        };
        let stored_commit = OnDiskCommit::new(commit, output_dir, working_copy_dir, tree_state);
        results.push(Arc::new(stored_commit));
    }
    Ok(results)
}

/// Get the shell to execute in and its first argument.
// TODO: use something like `[run].shell`
fn get_shell_executable_with_first_arg() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd", "/c")
    } else {
        ("/bin/sh", "-c")
    }
}

/// The result of a single command invocation.
struct RunJob {
    /// The old `CommitId` of the commit.
    old_id: CommitId,
    /// The new tree generated from the commit.
    new_tree: Tree,
    /// Was the tree even modified.
    dirty: bool,
}

// TODO: make this more revset/commit stream friendly.
async fn run_inner(
    tx: &WorkspaceCommandTransaction<'_>,
    sender: Sender<RunJob>,
    handle: &tokio::runtime::Handle,
    shell_command: Arc<String>,
    commits: Arc<Vec<Arc<OnDiskCommit>>>,
) -> Result<(), RunError> {
    let mut command_futures = JoinSet::new();
    for commit in commits.iter() {
        command_futures.spawn_on(
            rewrite_commit(
                // TODO: handle error here
                tx.base_workspace_helper().base_ignores().unwrap().clone(),
                ConflictMarkerStyle::Snapshot,
                commit.clone(),
                shell_command.clone(),
            ),
            handle,
        );
    }

    while let Some(res) = command_futures.join_next().await {
        let done = match res {
            Ok(rj) => rj?,
            Err(err) => return Err(RunError::JobFailure(err)),
        };
        let should_quit = sender.send(done).is_err();
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

/// Rewrite a single `OnDiskCommit`. The caller is responsible for creating the
/// final commit.
async fn rewrite_commit(
    base_ignores: Arc<GitIgnoreFile>,
    conflict_marker_style: ConflictMarkerStyle,
    stored_commit: Arc<OnDiskCommit>,
    shell_command: Arc<String>,
) -> Result<RunJob, RunError> {
    let (stdout, stderr) =
        create_output_files(stored_commit.commit.id(), &stored_commit.output_dir)?;
    // TODO: Later this should take some trait which allows `run` to integrate with
    // something like Bazels RE protocol.
    // e.g
    // ```
    // let mut executor /* Arc<dyn CommandExecutor> */ = store.get_executor();
    // let command = executor.spawn(...)?; // RE or separate processes depending on impl.
    // ...
    // ```
    tracing::debug!(
        "trying to run {} on commit {}",
        shell_command.as_str(),
        stored_commit.commit.id(),
    );
    let (prog, first_arg) = get_shell_executable_with_first_arg();
    let mut command = tokio::process::Command::new(prog)
        .arg(first_arg)
        .arg(shell_command.as_str())
        // set cwd to the working copy directory.
        .current_dir(&stored_commit.working_copy_dir)
        // .arg()
        // TODO: relativize
        // .env("JJ_PATH", stored_commit.working_copy_dir)
        .env("JJ_CHANGE", stored_commit.commit.change_id().hex())
        .env("JJ_COMMIT_ID", stored_commit.commit.id().hex())
        .stdout(stdout)
        .stderr(stderr)
        .kill_on_drop(true) // No zombies allowed.
        .spawn()?;

    let commit = stored_commit.commit.clone();
    let old_id = commit.id().clone();

    let status = command.wait().await?;

    if !status.success() {
        return Err(RunError::CommandFailure(status, old_id.clone()));
    }

    let tree_state = &mut stored_commit.tree_state.lock().await;

    let options = SnapshotOptions {
        base_ignores,
        conflict_marker_style,
        // TODO: read from current wc/settings
        start_tracking_matcher: &EverythingMatcher,
        fsmonitor_settings: FsmonitorSettings::None,
        progress: None,
        // TODO: read from current wc/settings
        max_new_file_size: 64_000_u64, // 64 MB for now,
    };
    tracing::debug!("trying to snapshot the new tree");
    let (dirty, _) = tree_state.snapshot(&options).unwrap();
    if !dirty {
        tracing::debug!(
            "commit {} was not modified as the passed command did not modify any tracked files",
            commit.id()
        );
    }

    let rewritten_id = tree_state.current_tree_id().to_merge();
    let new_id = rewritten_id.as_resolved().unwrap();

    let new_tree = commit
        .store()
        .get_tree_async(RepoPathBuf::root(), new_id)
        .await?;

    // TODO: Serialize the new tree into /output/{id-tree}

    Ok(RunJob {
        old_id,
        new_tree,
        dirty,
    })
}

/// Run a command across a set of revisions.
///
///
/// All recorded state will be persisted in the `.jj` directory, so occasionally
/// a `jj run --clean` is needed to clean up disk space.
///
/// # Example
///
/// # Run pre-commit on your local work
/// ```shell
/// $ jj run 'pre-commit run .github/pre-commit.yaml' -r (trunk()..@) -j 4
/// ```
///
/// This allows pre-commit integration and other funny stuff.
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub struct RunArgs {
    /// The command to run across all selected revisions.
    shell_command: String,
    /// The revisions to change.
    #[arg(long, short, default_value = "@", value_name = "REVSETS")]
    revisions: RevisionArg,
    /// A no-op option to match the interface of `git rebase -x`.
    #[arg(short = 'x', hide = true)]
    exec: bool,
    /// How many processes should run in parallel, uses by default all cores.
    #[arg(long, short)]
    jobs: Option<usize>,
}

pub fn cmd_run(ui: &mut Ui, command: &CommandHelper, args: &RunArgs) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    // The commits are already returned in reverse topological order.
    let resolved_commits: Vec<_> = workspace_command
        .parse_revset(ui, &args.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;

    workspace_command.check_rewritable(resolved_commits.iter().ids())?;
    // Jobs are resolved in this order:
    // 1. Commandline argument iff > 0.
    // 2. the amount of cores available.
    // 3. a single job, if all of the above fails.
    let jobs = match args.jobs {
        Some(0) | None => std::thread::available_parallelism().map(|t| t.into()).ok(),
        Some(jobs) => Some(jobs),
    }
    // Fallback to a single user-visible job.
    .unwrap_or(1usize);

    let rt = get_runtime(jobs);
    // TODO: Add a extension point for custom output/status aggregation.
    let mut done_commits = HashSet::new();
    let (sender_tx, receiver) = std::sync::mpsc::channel();

    let mut tx = workspace_command.start_transaction();
    let repo_path = tx.base_workspace_helper().repo_path();

    // TODO: consider on-demand creation for the inner loop.
    let stored_commits = Arc::new(create_working_copies(repo_path, &resolved_commits)?);
    let stored_len = stored_commits.len();

    let shell_command = args.shell_command.clone();
    // Start all the jobs.
    async {
        run_inner(
            &tx,
            sender_tx,
            rt.handle(),
            Arc::new(shell_command.clone()),
            stored_commits,
        )
        .await
    }
    .block_on()?;

    let mut rewritten_commits = HashMap::new();
    let mut visited = 0;
    loop {
        if let Ok(res) = receiver.recv() {
            // The tree was not changed, so ignore the result
            if !res.dirty {
                visited += 1;
                continue;
            }
            done_commits.insert(res.old_id.clone());
            rewritten_commits.insert(res.old_id.clone(), res.new_tree);
            visited += 1;
        }
        if visited == stored_len {
            break;
        }
    }
    drop(receiver);

    let run_path = repo_path.parent().unwrap().join("run").join("default");
    // The operation was a no-op, bail.
    if rewritten_commits.is_empty() {
        // Yeet everything, caching is better implemented in a follow-up.
        fs::remove_dir_all(&run_path)?;

        println!("No commits were rewritten as the command did not modify any tracked files");
        tx.finish(
            ui,
            format!("run: No-op on {visited} commits with {shell_command}"),
        )?;
        return Ok(());
    }

    // The command did something, so rewrite the commits.
    let mut count: u32 = 0;
    // TODO: handle the `--reparent` case here.
    tx.repo_mut().transform_descendants(
        resolved_commits.iter().ids().cloned().collect_vec(),
        |rewriter| {
            let old_id = rewriter.old_commit().id();
            let new_tree = rewritten_commits.get(old_id).unwrap();
            let new_tree_id = new_tree.id().clone();
            count += 1;
            let builder = rewriter.rebase()?;
            builder
                .set_tree_id(MergedTreeId::resolved(new_tree_id))
                .write()?;
            Ok(())
        },
    )?;
    println!("Rewrote {count} commits with {shell_command}");

    // Yeet everything, caching is better implemented in a follow-up.
    fs::remove_dir_all(&run_path)?;

    tx.finish(
        ui,
        format!("run: rewrite {count} commits with {shell_command}"),
    )?;

    Ok(())
}
