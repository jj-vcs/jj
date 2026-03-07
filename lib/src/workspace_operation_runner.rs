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

//! Contains the [`WorkspaceOperationRunner`] and associated helpers needed to
//! start mutable actions on a [`Transaction`].

use std::cell::OnceCell;
use std::error::Error;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use indexmap::IndexSet;
use itertools::Itertools as _;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::backend::BackendError;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::fileset;
use crate::fileset::FilesetDiagnostics;
use crate::fileset::FilesetExpression;
use crate::git::GitExportError;
use crate::git::GitExportStats;
#[cfg(feature = "git")]
use crate::git::GitImportError;
#[cfg(feature = "git")]
use crate::git::GitImportOptions;
#[cfg(feature = "git")]
use crate::git::GitImportStats;
use crate::git::GitResetHeadError;
use crate::git::export_refs;
use crate::git::update_intent_to_add;
use crate::id_prefix::IdPrefixContext;
#[cfg(feature = "git")]
use crate::merged_tree::MergedTree;
use crate::op_store::OpStoreError;
use crate::op_store::OperationId;
use crate::op_walk::OpsetEvaluationError;
use crate::op_walk::resolve_op_with_repo;
use crate::operation::Operation;
use crate::readonly_user_repo::ReadonlyUserRepo;
use crate::ref_name::WorkspaceName;
use crate::repo::CheckOutCommitError;
use crate::repo::EditCommitError;
use crate::repo::MutableRepo;
use crate::repo::ReadonlyRepo;
use crate::repo::Repo as _;
use crate::repo::RepoLoaderError;
use crate::repo::RewriteRootCommit;
use crate::revset;
use crate::revset::RevsetDiagnostics;
use crate::revset::RevsetExpression;
use crate::revset::UserRevsetExpression;
use crate::revset_util::RevsetExpressionEvaluator;
use crate::settings::UserSettings;
use crate::transaction::Transaction;
use crate::transaction::TransactionCommitError;
use crate::user_error::UserError;
use crate::user_error::short_operation_hash;
use crate::user_error::user_error;
use crate::working_copy;
use crate::working_copy::CheckoutError;
use crate::working_copy::CheckoutStats;
use crate::working_copy::LockedWorkingCopy;
use crate::working_copy::RecoverWorkspaceError;
#[cfg(feature = "git")]
use crate::working_copy::ResetError;
use crate::working_copy::SnapshotError;
use crate::working_copy::SnapshotOptions;
use crate::working_copy::SnapshotStats;
use crate::working_copy::WorkingCopyFreshness;
use crate::working_copy::WorkingCopyStateError;
use crate::workspace::Workspace;
use crate::workspace_util::WorkspaceEnvironment;

/// A wrapper for all common errors in this module.
// TODO: I find this quite ugly but maybe Yuya has a better idea.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceOperationError {
    /// An error during a backend action occurred.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// An error occurred during the Git export.
    #[error(transparent)]
    GitExport(#[from] GitExportError),
    /// An error occurred during the Git reset.
    #[error(transparent)]
    GitReset(#[from] GitResetHeadError),
    /// An error occurred during the read from the OperationStore.
    #[error(transparent)]
    OperationStore(#[from] OpStoreError),
    /// An error occurred during the recovery.
    #[error(transparent)]
    RecoverWorkspace(#[from] RecoverWorkspaceError),
    /// An error occurred during the load from the repo.
    #[error(transparent)]
    RepoLoad(RepoLoaderError),
    /// An error occurred during the rewrite, as we tried to rewrite the virtual
    /// root commit.
    #[error(transparent)]
    RewriteRoot(#[from] RewriteRootCommit),
    /// An error occurred during the snapshot.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// The working copy is stale
    #[error("The workspace is stale (at operation {}).", short_operation_hash(_0))]
    StaleWorkingCopy(OperationId),
    /// An error occurred while committing the transaction.
    #[error(transparent)]
    Transaction(#[from] TransactionCommitError),
    /// An error in the working-copy state occurred.
    #[error(transparent)]
    WorkingCopyState(#[from] WorkingCopyStateError),
    /// The workspace is stale and a sibling operation exists.
    #[error(
        "The workspace is stale (at operation {}) with the sibling being {}.",
        short_operation_hash(_0),
        short_operation_hash(_1)
    )]
    WorkspaceStaleSibling(OperationId, OperationId),
}

/// Reflects the state after calling
/// [`WorkspaceOperationRunner::finish_transaction`].
pub struct FinishedTransactionState {
    /// The number of revisions which got rebased, when finishing the
    /// transaction.
    pub num_rebased: usize,
    /// The stats from running the Git export steps.
    pub git_export_stats: GitExportStats,
    /// An additional error source from the immutable commit resolution, if
    /// there's any.
    // TODO: this is a wonky API but needs to exist to preserve CLI level diagnostics.
    pub immutable_err: Option<Arc<dyn Error + Send + Sync + 'static>>,
    /// An optional error source from the Git, only set when we failed to reset
    /// the Git HEAD.
    // TODO: this is a wonky API but needs to exist to preserve CLI level diagnostics.
    pub git_reset_err: Option<Box<dyn Error + Send + Sync + 'static>>,
    /// If finishing the transaction required moving of an now immutable
    /// revision.
    pub moved_off_immutable: bool,
    /// The optional checkout stats, if the finishing the transaction updated
    /// the `Workspace`'s working copy.
    pub checkout_stats: CheckoutStats,
    /// The new working copy commit if necessary.
    pub maybe_new_wc_commit: Option<Commit>,
    /// Set if the transaction was finished but no username was set in the
    /// config.
    pub missing_user_name: bool,
    /// Set if the transaction was finished but no email was set in the config.
    pub missing_user_mail: bool,
}

impl FinishedTransactionState {
    fn new() -> Self {
        Self {
            num_rebased: 0,
            git_export_stats: GitExportStats {
                failed_tags: Vec::new(),
                failed_bookmarks: Vec::new(),
            },
            moved_off_immutable: false,
            immutable_err: None,
            git_reset_err: None,
            checkout_stats: CheckoutStats::default(),
            maybe_new_wc_commit: None,
            missing_user_name: false,
            missing_user_mail: false,
        }
    }
}

/// A type which encompasses all errors which can occur when calling
/// [`WorkspaceOperationRunner::finish_transaction`].
#[derive(Debug, thiserror::Error)]
pub enum FinishedTransactionError {
    /// An error occurred in the backend.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// An error occurred when checking out a commit.
    #[error(transparent)]
    Checkout(#[from] CheckoutError),
    /// An error occurred when checking out a commit.
    #[error(transparent)]
    CheckoutCommit(#[from] CheckOutCommitError),
    /// An error occurred when optionally exporting to Git.
    #[error(transparent)]
    GitExport(#[from] GitExportError),
    /// An error occurred when we tried to reset the Git HEAD.
    #[error(transparent)]
    ResetHeadFailed(#[from] GitResetHeadError),
    /// An error occurred when committing the Transaction to the Operation Log.
    #[error(transparent)]
    Transaction(#[from] TransactionCommitError),
}

/// A type which encompasses all errors which can occur when calling
/// [`WorkspaceOperationRunner::import_git_head]`. Only usable when Git is
/// compiled in.
#[cfg(feature = "git")]
#[derive(Debug, thiserror::Error)]
pub enum ImportGitHeadError {
    /// An error occurred when acting on the backend.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// An error occurred when checking out the new commit.
    #[error(transparent)]
    Checkout(#[from] CheckoutError),
    /// An error occurred when checking out the new HEAD.
    #[error(transparent)]
    CheckoutCommit(#[from] CheckOutCommitError),
    /// An error occurred when exporting to Git.
    #[error(transparent)]
    GitExport(#[from] GitExportError),
    /// An error occurred when running the actual Git import.
    #[error(transparent)]
    GitImport(#[from] GitImportError),
    /// An error occurred when resetting the Git HEAD ref.
    #[error(transparent)]
    GitReset(#[from] GitResetHeadError),
    /// An error occurred during Git HEAD reset after the import.
    #[error(transparent)]
    Reset(#[from] ResetError),
    /// An error occurred when finishing the underlying transaction.
    #[error(transparent)]
    TransactionCommit(#[from] TransactionCommitError),
    /// An error occurred during the re-reading of the working-copy state.
    #[error(transparent)]
    WorkingCopyState(#[from] WorkingCopyStateError),
}

#[cfg(feature = "git")]
impl From<FinishedTransactionError> for ImportGitHeadError {
    fn from(value: FinishedTransactionError) -> Self {
        match value {
            FinishedTransactionError::Backend(backend_error) => Self::Backend(backend_error),
            FinishedTransactionError::Checkout(checkout_error) => Self::Checkout(checkout_error),
            FinishedTransactionError::CheckoutCommit(check_out_commit_error) => {
                Self::CheckoutCommit(check_out_commit_error)
            }
            FinishedTransactionError::GitExport(git_export_error) => {
                Self::GitExport(git_export_error)
            }
            FinishedTransactionError::ResetHeadFailed(git_reset_head_error) => {
                Self::GitReset(git_reset_head_error)
            }
            FinishedTransactionError::Transaction(transaction_commit_error) => {
                Self::TransactionCommit(transaction_commit_error)
            }
        }
    }
}

/// Reflects the state after a [`WorkspaceOperationRunner::import_git_refs`]
/// call, only visible when Git support is compiled in.
#[cfg(feature = "git")]
pub struct ImportGitState {
    /// The information from the underlying Git import call.
    pub import_stats: GitImportStats,
    /// The number of revisions this moved after the Git import.
    pub num_rebased: usize,
}

/// Represents all errors which can occur in a
/// [`WorkspaceOperationRunner::import_git_refs`] call. Only visible
/// when Git support is compiled in.
#[cfg(feature = "git")]
#[derive(Debug, thiserror::Error)]
pub enum ImportGitError {
    /// An error occurred during a Backend interaction.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// An error occurred when we tried to update the workspace to a new commit.
    #[error(transparent)]
    Checkout(#[from] CheckoutError),
    /// An error occurred during the checkout of a commit when finishing the
    /// transaction.
    #[error(transparent)]
    CheckoutCommit(#[from] CheckOutCommitError),
    /// An error occurred when exporting to Git at the end of a transaction.
    #[error(transparent)]
    GitExport(#[from] GitExportError),
    /// An error occurred during the actual Git import.
    #[error(transparent)]
    GitImport(#[from] GitImportError),
    /// An error occurred when we tried to reset the Git HEAD ref.
    #[error(transparent)]
    ResetFailed(#[from] GitResetHeadError),
    /// An error occurred when trying to commit the underlying transaction.
    #[error(transparent)]
    Transaction(#[from] TransactionCommitError),
}

#[cfg(feature = "git")]
impl From<FinishedTransactionError> for ImportGitError {
    fn from(value: FinishedTransactionError) -> Self {
        match value {
            FinishedTransactionError::Backend(backend_error) => Self::Backend(backend_error),
            FinishedTransactionError::Checkout(checkout_error) => Self::Checkout(checkout_error),
            FinishedTransactionError::CheckoutCommit(check_out_commit_error) => {
                Self::CheckoutCommit(check_out_commit_error)
            }
            FinishedTransactionError::GitExport(git_export_error) => {
                Self::GitExport(git_export_error)
            }
            FinishedTransactionError::ResetHeadFailed(git_reset_head_error) => {
                Self::ResetFailed(git_reset_head_error)
            }
            FinishedTransactionError::Transaction(transaction_commit_error) => {
                Self::Transaction(transaction_commit_error)
            }
        }
    }
}

/// Contains all information after a [`WorkspaceOperationRunner::snapshot`]
/// call.
#[derive(Default)]
pub struct SnapshotState {
    /// The stats from the new snapshot.
    pub stats: SnapshotStats,
    /// The number of revisions rebased by snapshotting.
    pub num_rebased: usize,
    /// Whether the Workspace has `.jjconflict` files. This is ignored (false)
    /// if the workspace is not shared with Git or Git support is not
    /// compiled in.
    pub has_jj_conflict_files: bool,
}

/// TODO: A `WorkspaceOperationRunner is ...?
pub struct WorkspaceOperationRunner {
    /// The `WorkspaceEnvironment` associated with this runner.
    env: WorkspaceEnvironment,
    /// The `Workspace` we're currently operating on.
    workspace: Workspace,
    /// The `ReadonlyUserRepo` which we're currently operating on.
    user_repo: ReadonlyUserRepo,
}

impl WorkspaceOperationRunner {
    /// Creates a new `WorkspaceOperationRunner`.
    pub fn new(
        env: WorkspaceEnvironment,
        workspace: Workspace,
        user_repo: ReadonlyUserRepo,
    ) -> Self {
        Self {
            env,
            workspace,
            user_repo,
        }
    }

    /// Gets the associated `WorkspaceEnvironment`
    pub fn env(&self) -> &WorkspaceEnvironment {
        &self.env
    }

    /// Gets the associated `Workspace`.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Gets a mutable reference to the associated `Workspace`.
    // TODO: remove if possible
    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    /// Returns the name of the associated `Workspace`.
    pub fn workspace_name(&self) -> &WorkspaceName {
        self.workspace.workspace_name()
    }

    /// Returns the `Settings` of the associated `Workspace`
    pub fn settings(&self) -> &UserSettings {
        self.workspace.settings()
    }

    /// Gets the associated `ReadonlyUserRepo`.
    pub fn user_repo(&self) -> &ReadonlyUserRepo {
        &self.user_repo
    }

    /// Gets the associated `ReadOnlyRepo`
    pub fn read_only_repo(&self) -> &Arc<ReadonlyRepo> {
        self.user_repo.repo()
    }

    /// Gets a mutable reference to the associated `ReadonlyUserRepo`.
    // TODO: remove if possible
    pub fn user_repo_mut(&mut self) -> &mut ReadonlyUserRepo {
        &mut self.user_repo
    }
    /// Updates the `ReadonlyUserRepo` to be `repo`.
    pub fn update_user_repo(&mut self, repo: ReadonlyUserRepo) {
        self.user_repo = repo;
    }

    /// Resolves single operation for the given `op_str`.
    pub async fn resolve_single_op(&self, op_str: &str) -> Result<Operation, OpsetEvaluationError> {
        resolve_op_with_repo(self.user_repo.repo(), op_str).await
    }

    /// Gets the current working-copy's `CommitId` for the associated repo.
    pub fn get_wc_commit_id(&self) -> Option<&CommitId> {
        self.read_only_repo()
            .view()
            .get_wc_commit_id(self.workspace_name())
    }

    /// Resolves a revset to a single revision.
    pub async fn resolve_single_rev(
        &self,
        revision_arg: &str,
        diagnostics: &mut RevsetDiagnostics,
    ) -> Result<Commit, UserError> {
        let expression = self.parse_revset(revision_arg, diagnostics)?;
        // This is a copy of `jj_cli::cli_util::resolve_single_rev`'s body without the
        // callback taking a TemplateRenderer
        let commits: Vec<_> = expression
            .evaluate_to_commits()?
            .take(6)
            .try_collect()
            .await?;
        match commits.as_slice() {
            [commit] => Ok(commit.clone()),
            [] => Err(user_error(format!(
                "Revset `{revision_arg}` didn't resolve to any revisions"
            ))),
            _ => Err(user_error(format!(
                "Revset `{revision_arg}` resolved to more than one revision"
            ))),
        }
    }

    /// Parses the given strings as file patterns.
    pub fn parse_file_patterns(
        &self,
        values: &[String],
        diagnostics: &mut FilesetDiagnostics,
    ) -> Result<FilesetExpression, UserError> {
        // TODO: This function might be superseded by parse_union_filesets(),
        // but it would be weird if parse_union_*() had a special case for the
        // empty arguments.
        if values.is_empty() {
            Ok(FilesetExpression::all())
        } else {
            self.parse_union_filesets(values, diagnostics)
        }
    }

    /// Parses the fileset expressions in `file_args` and concatenates them all.
    pub fn parse_union_filesets(
        &self,
        file_args: &[String], // TODO: introduce FileArg newtype?
        diagnostics: &mut FilesetDiagnostics,
    ) -> Result<FilesetExpression, UserError> {
        let context = self.env.fileset_parse_context();
        let expressions: Vec<_> = file_args
            .iter()
            .map(|arg| fileset::parse_maybe_bare(diagnostics, arg, &context))
            .try_collect()?;
        Ok(FilesetExpression::union_all(expressions))
    }

    /// Evaluates revset expressions to a set of commit IDs. The
    /// returned set preserves the order of the input expressions.
    pub async fn resolve_revsets_ordered(
        &self,
        revision_args: &[&str],
        diagnostics: &mut RevsetDiagnostics,
    ) -> Result<IndexSet<CommitId>, UserError> {
        let mut all_commits = IndexSet::new();
        for revision_arg in revision_args {
            let expression = self.parse_revset(revision_arg, diagnostics)?;
            let mut stream = expression.evaluate_to_commit_ids()?;
            while let Some(commit_id) = stream.try_next().await? {
                all_commits.insert(commit_id);
            }
        }
        Ok(all_commits)
    }

    /// Evaluates revset expressions to a non-empty set of commit IDs. The
    /// returned set preserves the order of the input expressions.
    pub async fn resolve_some_revsets(
        &self,
        revision_args: &[&str],
        mut diagnostics: RevsetDiagnostics,
    ) -> Result<IndexSet<CommitId>, UserError> {
        let all_commits = self
            .resolve_revsets_ordered(revision_args, &mut diagnostics)
            .await?;
        if all_commits.is_empty() {
            Err(user_error("Empty revision set"))
        } else {
            Ok(all_commits)
        }
    }

    /// Parses a single revset in `revision_arg` and returns an
    /// `RevsetEvaluator` for further use. All diagnostics are written to
    /// `diagnostics`.
    pub fn parse_revset(
        &self,
        revision_arg: &str,
        diagnostics: &mut RevsetDiagnostics,
    ) -> Result<RevsetExpressionEvaluator<'_>, UserError> {
        let context = self.env.revset_parse_context();
        let expression = revset::parse(diagnostics, revision_arg, &context)?;
        Ok(self.attach_revset_evaluator(expression))
    }

    /// Parses the given revset expressions and concatenates them all.
    pub fn parse_union_revsets(
        &self,
        revision_args: &[&str],
        diagnostics: &mut RevsetDiagnostics,
    ) -> Result<RevsetExpressionEvaluator<'_>, UserError> {
        let context = self.env.revset_parse_context();
        let expressions: Vec<_> = revision_args
            .iter()
            .map(|arg| revset::parse(diagnostics, arg, &context))
            .try_collect()?;
        let expression = RevsetExpression::union_all(&expressions);
        Ok(self.attach_revset_evaluator(expression))
    }

    /// Attaches a `RevsetExpressionEvaluator` to the given `expression` for
    /// further evaluations.
    pub fn attach_revset_evaluator(
        &self,
        expression: Arc<UserRevsetExpression>,
    ) -> RevsetExpressionEvaluator<'_> {
        RevsetExpressionEvaluator::new(
            self.user_repo.repo().as_ref(),
            self.env.revset_extensions().clone(),
            self.id_prefix_context(),
            expression,
        )
    }

    /// Gets or creates a new `IdPrefixContext` for this
    /// `WorkspaceOperationRunner`.
    pub fn id_prefix_context(&self) -> &IdPrefixContext {
        self.user_repo
            .id_prefix_context()
            .get_or_init(|| self.env.new_id_prefix_context())
    }

    /// Imports new HEAD from the colocated Git repo.
    ///
    /// If the Git HEAD has changed, this function checks out the new Git HEAD.
    /// The old working-copy commit will be abandoned if it's discardable. The
    /// working-copy state will be reset to point to the new Git HEAD. The
    /// working-copy contents won't be updated.
    ///
    /// Returns `true` if the old Git was present, `false` otherwise.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    pub async fn import_git_head(
        &mut self,
        args: &[String],
        may_update_working_copy: bool,
        working_copy_shared_with_git: bool,
        should_publish_transaction: bool,
        ignore_immutable: bool,
    ) -> Result<bool, ImportGitHeadError> {
        use jj_lib::git::import_head;

        let mut tx = self.start_transaction(args);
        import_head(tx.repo_mut()).await?;
        if !tx.repo().has_changes() {
            return Ok(false);
        }

        let mut tx = tx.into_inner();
        let old_git_head = self.user_repo.repo().view().git_head().clone();
        let new_git_head = tx.repo().view().git_head().clone();
        if let Some(new_git_head_id) = new_git_head.as_normal() {
            let workspace_name = self.workspace_name().to_owned();
            let new_git_head_commit = tx.repo().store().get_commit_async(new_git_head_id).await?;

            let wc_commit = tx
                .repo_mut()
                .check_out(workspace_name, &new_git_head_commit)
                .await?;

            let mut locked_ws = self.workspace.start_working_copy_mutation()?;

            // The working copy was presumably updated by the git command that updated
            // HEAD, so we just need to reset our working copy
            // state to it without updating working copy files.
            locked_ws.locked_wc().reset(&wc_commit).await?;

            tx.repo_mut().rebase_descendants().await?;

            self.user_repo = ReadonlyUserRepo::new(
                tx.maybe_publish("import git head", should_publish_transaction)
                    .await?,
            );
            locked_ws
                .finish(self.user_repo.repo().op_id().clone())
                .await?;

            return Ok(old_git_head.is_present());
        } else {
            // Unlikely, but the HEAD ref got deleted by git?
            self.finish_transaction(
                tx,
                "import git head",
                may_update_working_copy,
                working_copy_shared_with_git,
                should_publish_transaction,
                ignore_immutable,
            )
            .await?;
        }
        Ok(false)
    }

    /// Imports branches and tags from the underlying Git repo, abandons old
    /// bookmarks.
    ///
    /// If the working-copy branch is rebased, and if update is allowed, the
    /// new working-copy commit will be checked out.
    ///
    /// This function does not import the Git HEAD, but the HEAD may be reset to
    /// the working copy parent if the repository is colocated.
    ///
    /// Returns `None` if importing refs was a no-op. Otherwise the state
    /// contains both the Import stats and the number of revisions this
    /// rebased.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    pub async fn import_git_refs(
        &mut self,
        args: &[String],
        import_options: &GitImportOptions,
        may_update_working_copy: bool,
        working_copy_shared_with_git: bool,
        may_snapshot_working_copy: bool,
        ignore_immutable: bool,
    ) -> Result<Option<ImportGitState>, ImportGitError> {
        use jj_lib::git;
        let mut tx = self.start_transaction(args);
        let import_stats = git::import_refs(tx.repo_mut(), import_options).await?;
        if !tx.repo().has_changes() {
            return Ok(None);
        }

        let mut tx = tx.into_inner();
        // Rebase here to show slightly different status message.
        let num_rebased = tx.repo_mut().rebase_descendants().await?;
        self.finish_transaction(
            tx,
            "import git refs",
            may_update_working_copy,
            working_copy_shared_with_git,
            may_snapshot_working_copy,
            ignore_immutable,
        )
        .await?;
        let state = ImportGitState {
            import_stats,
            num_rebased,
        };
        Ok(Some(state))
    }

    /// Starts a new `WorkspaceOperationTransaction` on the given `Workspace`,
    /// `workspace_name` is used to trace from where this transaction stems.
    // TODO: maybe its possible to remove `args` here?
    // because non-CLI use-cases may not need to escape an operations tags
    pub fn start_transaction(&mut self, args: &[String]) -> WorkspaceOperationTransaction {
        let workspace_name = self.workspace_name();
        let tx = start_repo_transaction(self.user_repo.repo(), workspace_name, args);
        let id_prefix_context = self.user_repo.take_id_prefix_context();
        WorkspaceOperationTransaction::new(tx, id_prefix_context)
    }

    /// Snapshots the working-copy for the associated Workspace with the passed
    /// `options`. `args` are passed to the transaction this internally
    /// uses. If `working_copy_shared_with_git` is true and the library is
    /// compiled with Git support it also updates the underlying Git state.
    #[instrument(skip_all)]
    pub async fn snapshot_working_copy(
        &mut self,
        options: &SnapshotOptions<'_>,
        args: &[String],
        working_copy_shared_with_git: bool,
        may_snapshot_working_copy: bool,
    ) -> Result<SnapshotState, WorkspaceOperationError> {
        let workspace_name = self.workspace_name().to_owned();
        let repo = self.user_repo.repo().clone();
        let mut state = SnapshotState::default();

        // Compare working-copy tree and operation with repo's, and reload as needed.
        let mut locked_ws = self.workspace.start_working_copy_mutation()?;

        let Some((repo, wc_commit)) =
            handle_stale_working_copy(locked_ws.locked_wc(), repo, &workspace_name)
                .await
                .map_err(|e| e.into_workspace_operation_error())?
        else {
            // If the workspace has been deleted, it's unclear what to do, so we just skip
            // committing the working copy.
            return Ok(SnapshotState::default());
        };

        self.user_repo = ReadonlyUserRepo::new(repo);
        let (new_tree, stats) = locked_ws.locked_wc().snapshot(options).await?;
        state.stats = stats;
        if new_tree.tree_ids_and_labels() != wc_commit.tree().tree_ids_and_labels() {
            let mut tx = start_repo_transaction(self.user_repo.repo(), &workspace_name, args);
            tx.set_is_snapshot(true);
            let mut_repo = tx.repo_mut();
            let commit = mut_repo
                .rewrite_commit(&wc_commit)
                .set_tree(new_tree.clone())
                .write()
                .await?;
            mut_repo.set_wc_commit(workspace_name, commit.id().clone())?;

            // Rebase descendants
            state.num_rebased = mut_repo.rebase_descendants().await?;

            #[cfg(feature = "git")]
            if working_copy_shared_with_git && may_snapshot_working_copy {
                let old_tree = wc_commit.tree();
                let new_tree = commit.tree();
                export_working_copy_changes_to_git(mut_repo, &old_tree, &new_tree)
                    .await
                    .map_err(|e| e.into_workspace_operation_error())?;
            }

            let repo = tx
                .maybe_publish("snapshot working copy", may_snapshot_working_copy)
                .await?;
            self.user_repo = ReadonlyUserRepo::new(repo);
        }

        #[cfg(feature = "git")]
        let has_jj_conflict_files = if working_copy_shared_with_git
            && let Ok(resolved_tree) = new_tree.trees().await?.into_resolved()
            && resolved_tree
                .entries_non_recursive()
                .any(|entry| entry.name().as_internal_str().starts_with(".jjconflict"))
        {
            true
        } else {
            false
        };
        state.has_jj_conflict_files = has_jj_conflict_files;

        locked_ws
            .finish(self.user_repo.repo().op_id().clone())
            .await?;
        Ok(state)
    }

    /// Updates this `WorkspaceOperationRunner` to `new_commit` calculating all
    /// deletions and additions.
    pub async fn update_working_copy(
        &mut self,
        maybe_old_commit: Option<&Commit>,
        new_commit: &Commit,
    ) -> Result<CheckoutStats, CheckoutError> {
        let stats = update_working_copy(
            self.user_repo.repo(),
            &mut self.workspace,
            maybe_old_commit,
            new_commit,
        )
        .await?;
        Ok(stats)
    }

    /// Creates a new `Commit` and make it the checked out commit in the
    /// associated Workspace.
    pub async fn create_and_check_out_recovery_commit(
        &mut self,
        description: &str,
    ) -> Result<(Arc<ReadonlyRepo>, Commit), WorkspaceOperationError> {
        let workspace_name = self.workspace_name().to_owned();
        let mut locked_ws = self
            .workspace
            .start_working_copy_mutation()
            .map_err(WorkspaceOperationError::WorkingCopyState)?;
        let (repo, new_commit) = working_copy::create_and_check_out_recovery_commit(
            locked_ws.locked_wc(),
            self.user_repo.repo(),
            workspace_name,
            description,
        )
        .await?;

        locked_ws.finish(repo.op_id().clone()).await?;
        Ok((repo, new_commit))
    }

    /// Finishes a [`Transaction`] created by `start_transaction` with the given
    /// description. If `may_update_working_copy` is true it also returns
    /// the commit to update to if the commit the workspace was on turned
    /// out to be immutable. If `working_copy_shared_with_git` is true and
    /// Git support is compiled in we also export all changed refs and
    /// propagate the `GitExportStats` to callers.
    pub async fn finish_transaction(
        &mut self,
        mut tx: Transaction,
        description: impl Into<String>,
        may_update_working_copy: bool,
        working_copy_shared_with_git: bool,
        may_snapshot_working_copy: bool,
        ignore_immutable: bool,
    ) -> Result<(FinishedTransactionState, Arc<ReadonlyRepo>), FinishedTransactionError> {
        let mut state = FinishedTransactionState::new();
        state.num_rebased = tx.repo_mut().rebase_descendants().await?;

        for (name, wc_commit_id) in &tx.repo().view().wc_commit_ids().clone() {
            // This can fail if trunk() bookmark gets deleted or conflicted. If
            // the unresolvable trunk() issue gets addressed differently, it
            // should be okay to propagate the error.
            let wc_expr = RevsetExpression::commit(wc_commit_id.clone());
            let is_immutable = match self
                .env
                .find_immutable_commit(tx.repo(), &wc_expr, ignore_immutable)
                .await
            {
                Ok(commit_id) => commit_id.is_some(),
                Err(UserError { error, .. }) => {
                    // Give up because the same error would occur repeatedly.
                    state.immutable_err = Some(error.clone());
                    false
                }
            };
            if is_immutable {
                let wc_commit = tx.repo().store().get_commit_async(wc_commit_id).await?;
                tx.repo_mut().check_out(name.clone(), &wc_commit).await?;
                state.moved_off_immutable = true;
            }
        }

        let old_repo = tx.base_repo().clone();

        let maybe_old_wc_commit = old_repo
            .view()
            .get_wc_commit_id(self.workspace_name())
            .map(|commit_id| tx.base_repo().store().get_commit(commit_id))
            .transpose()?;
        state.maybe_new_wc_commit = tx
            .repo()
            .view()
            .get_wc_commit_id(self.workspace_name())
            .map(|commit_id| tx.repo().store().get_commit(commit_id))
            .transpose()?;

        #[cfg(feature = "git")]
        if working_copy_shared_with_git && may_snapshot_working_copy {
            if let Some(wc_commit) = &state.maybe_new_wc_commit {
                // Export Git HEAD while holding the git-head lock to prevent races:
                // - Between two finish_transaction calls updating HEAD
                // - With import_git_head importing HEAD concurrently
                // This can still fail if HEAD was updated concurrently by another JJ process
                // (overlapping transaction) or a non-JJ process (e.g., git checkout). In that
                // case, the actual state will be imported on the next snapshot.

                use crate::git::reset_head;

                match reset_head(tx.repo_mut(), wc_commit).await {
                    Ok(()) => {}
                    Err(err @ jj_lib::git::GitResetHeadError::UpdateHeadRef(_)) => {
                        state.git_reset_err = Some(Box::new(err));
                    }
                    Err(err) => return Err(FinishedTransactionError::ResetHeadFailed(err)),
                }
            }
            state.git_export_stats = export_refs(tx.repo_mut())?;
        }

        self.user_repo = ReadonlyUserRepo::new(
            tx.maybe_publish(description, may_update_working_copy)
                .await?,
        );

        // Update working copy before reporting repo changes, so that
        // potential errors while reporting changes (broken pipe, etc)
        // don't leave the working copy in a stale state.
        if may_update_working_copy {
            if let Some(new_commit) = &state.maybe_new_wc_commit {
                state.checkout_stats = self
                    .update_working_copy(maybe_old_wc_commit.as_ref(), new_commit)
                    .await?;
            } else {
                // It seems the workspace was deleted, so we shouldn't try to
                // update it.
            }
        }

        let settings = self.settings();
        state.missing_user_name = settings.user_name().is_empty();
        state.missing_user_mail = settings.user_email().is_empty();
        Ok((state, old_repo))
    }
}

/// An ongoing [`Transaction`] tied to a particular workspace.
///
/// `WorkspaceOperationTransaction`s are created with
/// [`WorkspaceOperationRunner::start_transaction`] and committed with
/// [`WorkspaceCommandTransaction::finish`]. The inner `Transaction` can also be
/// extracted using [`WorkspaceOperationTransaction::into_inner`] in situations
/// where finer-grained control over the `Transaction` is necessary.
///
/// This usually should be used as an inner type in your downstream's customized
/// Transaction type.
/// ```no_run
/// struct MyTransactionType {
///   inner: WorkspaceOperationTransaction
///   //...
/// }
/// ```
#[must_use]
pub struct WorkspaceOperationTransaction {
    /// The `Transaction` we operate on.
    tx: Transaction,
    /// Cache of index built against the current MutableRepo state.
    id_prefix_context: OnceCell<IdPrefixContext>,
}

impl WorkspaceOperationTransaction {
    /// Creates a new `WorkspaceOperationTransaction`.
    pub fn new(tx: Transaction, id_prefix_context: OnceCell<IdPrefixContext>) -> Self {
        Self {
            tx,
            id_prefix_context,
        }
    }

    /// Returns the base `ReadonlyRepo` within the `Transaction`.
    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        self.tx.base_repo()
    }

    /// Returns a reference to the `MutableRepo` used for this `Transaction`.
    pub fn repo(&self) -> &MutableRepo {
        self.tx.repo()
    }

    /// Returns a mutable reference to the `MutableRepo` used for this
    /// transaction.
    pub fn repo_mut(&mut self) -> &mut MutableRepo {
        self.id_prefix_context.take(); // invalidate
        self.tx.repo_mut()
    }

    /// Checks out the given `Commit`.
    // TODO: should be async
    pub fn check_out(
        &mut self,
        commit: &Commit,
        name: &WorkspaceName,
    ) -> Result<Commit, CheckOutCommitError> {
        self.id_prefix_context.take(); // invalidate
        self.tx
            .repo_mut()
            .check_out(name.to_owned(), commit)
            .block_on()
    }

    /// Edits the given `Commit`.
    // TODO: should be async
    pub fn edit(&mut self, commit: &Commit, name: &WorkspaceName) -> Result<(), EditCommitError> {
        self.id_prefix_context.take(); // invalidate
        self.tx.repo_mut().edit(name.to_owned(), commit).block_on()
    }

    /// Returns the wrapped [`Transaction`] for circumstances where
    /// finer-grained control is needed. The caller becomes responsible for
    /// finishing the `Transaction`, including rebasing descendants and updating
    /// the working copy, if applicable.
    // TODO: maybe rename this to `into_inner_transaction`
    pub fn into_inner(self) -> Transaction {
        self.tx
    }

    /// Gets the associated `IdPrefixContext`.
    pub fn id_prefix_context(&self) -> &OnceCell<IdPrefixContext> {
        &self.id_prefix_context
    }

    /// Finishes this `WorkspaceOperationTransaction` with `description` on the
    /// given `runner`.
    pub async fn finish(
        self,
        runner: &mut WorkspaceOperationRunner,
        description: impl Into<String>,
        may_update_working_copy: bool,
        working_copy_shared_with_git: bool,
        may_snapshot_working_copy: bool,
        ignore_immutable: bool,
    ) -> Result<(), FinishedTransactionError> {
        // no-op so bail early.
        if !self.repo().has_changes() {
            return Ok(());
        }

        runner
            .finish_transaction(
                self.tx,
                description,
                may_update_working_copy,
                working_copy_shared_with_git,
                may_snapshot_working_copy,
                ignore_immutable,
            )
            .await?;
        Ok(())
    }
}

/// Starts a new `Transaction` by doing the necessary shell-escaping.
pub fn start_repo_transaction(
    repo: &Arc<ReadonlyRepo>,
    workspace_name: &WorkspaceName,
    string_args: &[String],
) -> Transaction {
    let mut tx = repo.start_transaction();
    tx.set_workspace_name(workspace_name);
    // TODO: Either do better shell-escaping here or store the values in some list
    // type (which we currently don't have).
    let shell_escape = |arg: &String| {
        if arg.as_bytes().iter().all(|b| {
            matches!(b,
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b','
                | b'-'
                | b'.'
                | b'/'
                | b':'
                | b'@'
                | b'_'
            )
        }) {
            arg.clone()
        } else {
            format!("'{}'", arg.replace('\'', "\\'"))
        }
    };
    let mut quoted_strings = vec!["jj".to_string()];
    quoted_strings.extend(string_args.iter().skip(1).map(shell_escape));
    tx.set_attribute("args".to_string(), quoted_strings.join(" "));
    tx
}

/// Updates the `Workspace` to `new_commit` while calculating the cumulative
/// additions and deletions.
pub async fn update_working_copy(
    repo: &Arc<ReadonlyRepo>,
    workspace: &mut Workspace,
    old_commit: Option<&Commit>,
    new_commit: &Commit,
) -> Result<CheckoutStats, CheckoutError> {
    let old_tree = old_commit.map(|commit| commit.tree());
    // TODO: CheckoutError::ConcurrentCheckout should probably just result in a
    // warning for most commands (but be an error for the checkout command)
    let stats = workspace
        .check_out(repo.op_id().clone(), old_tree.as_ref(), new_commit)
        .await?;
    Ok(stats)
}

/// An error which can occur during the export to Git.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceGitExportError {
    /// We failed to reset the Git head.
    #[error(transparent)]
    GitReset(#[from] GitResetHeadError),
    /// We failed to export to Git.
    #[error(transparent)]
    GitExport(#[from] GitExportError),
}

impl WorkspaceGitExportError {
    /// Converts a `WorkspaceGitExportError` into a [`WorkspaceOperationError`].
    pub fn into_workspace_operation_error(self) -> WorkspaceOperationError {
        match self {
            Self::GitReset(e) => WorkspaceOperationError::GitReset(e),
            Self::GitExport(e) => WorkspaceOperationError::GitExport(e),
        }
    }
}

/// An error which encompasses all errors which could happen when handling a
/// stale working-copy.
#[derive(Debug, thiserror::Error)]
pub enum HandleStaleWorkingCopyError {
    /// We failed to get something from the backend.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// There was an error during an operation fetch.
    #[error(transparent)]
    OperationStoreError(#[from] OpStoreError),
    /// We failed to load the underlying repo.
    #[error(transparent)]
    RepoLoad(#[from] RepoLoaderError),
    /// The working copy is stale and a sibling operation exists.
    #[error(
        "The working-copy is stale at operation {} and a sibling operation exists {}",
        short_operation_hash(_0),
        short_operation_hash(_1)
    )]
    StaleSiblingOperation(OperationId, OperationId),
    /// The working copy is stale.
    #[error(
        "The working copy is stale (not updated since operation {}).",
        short_operation_hash(_0)
    )]
    WorkingCopyStale(OperationId),
}

impl HandleStaleWorkingCopyError {
    /// Convert a `HandleStaleWorkingCopyError` into a
    /// [`WorkspaceOperationError`].
    pub fn into_workspace_operation_error(self) -> WorkspaceOperationError {
        match self {
            Self::Backend(backend_error) => WorkspaceOperationError::Backend(backend_error),
            Self::OperationStoreError(op_store_error) => {
                WorkspaceOperationError::OperationStore(op_store_error)
            }
            Self::RepoLoad(repo_loader_error) => {
                WorkspaceOperationError::RepoLoad(repo_loader_error)
            }
            Self::StaleSiblingOperation(sibling_id, operation_id) => {
                WorkspaceOperationError::WorkspaceStaleSibling(sibling_id, operation_id)
            }
            Self::WorkingCopyStale(operation_id) => {
                WorkspaceOperationError::StaleWorkingCopy(operation_id)
            }
        }
    }
}

/// Checks if the working copy is stale and reloads the repo if the repo is
/// ahead of the working copy.
///
/// Returns Ok(None) if the workspace doesn't exist in the repo (presumably
/// because it was deleted).
// TODO: Maybe this shouldn't be exported.
pub async fn handle_stale_working_copy(
    locked_wc: &mut dyn LockedWorkingCopy,
    repo: Arc<ReadonlyRepo>,
    workspace_name: &WorkspaceName,
) -> Result<Option<(Arc<ReadonlyRepo>, Commit)>, HandleStaleWorkingCopyError> {
    let get_wc_commit = |repo: &ReadonlyRepo| -> Result<Option<_>, _> {
        repo.view()
            .get_wc_commit_id(workspace_name)
            .map(|id| repo.store().get_commit(id))
            .transpose()
    };
    let Some(wc_commit) = get_wc_commit(&repo)? else {
        return Ok(None);
    };
    let old_op_id = locked_wc.old_operation_id().clone();
    match WorkingCopyFreshness::check_stale(locked_wc, &wc_commit, &repo).await {
        Ok(WorkingCopyFreshness::Fresh) => Ok(Some((repo, wc_commit))),
        Ok(WorkingCopyFreshness::Updated(wc_operation)) => {
            let repo = repo.reload_at(&wc_operation).await?;
            if let Some(wc_commit) = get_wc_commit(&repo)? {
                Ok(Some((repo, wc_commit)))
            } else {
                Ok(None)
            }
        }
        Ok(WorkingCopyFreshness::WorkingCopyStale) => {
            Err(HandleStaleWorkingCopyError::WorkingCopyStale(old_op_id))
        }
        Ok(WorkingCopyFreshness::SiblingOperation) => Err(
            HandleStaleWorkingCopyError::StaleSiblingOperation(repo.op_id().clone(), old_op_id),
        ),
        Err(e @ OpStoreError::ObjectNotFound { .. }) => {
            Err(HandleStaleWorkingCopyError::OperationStoreError(e))
        }
        Err(e) => Err(HandleStaleWorkingCopyError::OperationStoreError(e)),
    }
}

/// Exports the changes from the working-copy to the underlying Git repo.
/// Adds all the changes calculated from `old_tree` to `new_tree` and update the
/// intent to add.
#[cfg(feature = "git")]
pub async fn export_working_copy_changes_to_git(
    mut_repo: &mut MutableRepo,
    old_tree: &MergedTree,
    new_tree: &MergedTree,
) -> Result<GitExportStats, WorkspaceGitExportError> {
    let repo = mut_repo.base_repo().as_ref();
    update_intent_to_add(repo, old_tree, new_tree).await?;
    let stats = export_refs(mut_repo)?;
    Ok(stats)
}

/// Exports the changes from the working-copy to the underlying Git repo.
#[cfg(not(feature = "git"))]
pub async fn export_working_copy_changes_to_git(
    _mut_repo: &mut MutableRepo,
    _old_tree: &MergedTree,
    _new_tree: &MergedTree,
) -> Result<GitExportStats, WorkspaceGitExportError> {
    Ok(GitExportStats::default())
}
