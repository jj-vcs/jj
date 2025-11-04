// Copyright 2023 The Jujutsu Authors
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

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use itertools::Itertools as _;
use jj_cli::cli_util::CliRunner;
use jj_cli::cli_util::CommandHelper;
use jj_cli::command_error::CommandError;
use jj_cli::ui::Ui;
use jj_lib::backend::Backend;
use jj_lib::commit::Commit;
use jj_lib::git_backend::GitBackend;
use jj_lib::local_working_copy::LocalWorkingCopy;
use jj_lib::merged_tree::MergedTree;
use jj_lib::op_store::OperationId;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::settings::UserSettings;
use jj_lib::signing::Signer;
use jj_lib::store::Store;
use jj_lib::working_copy::CheckoutError;
use jj_lib::working_copy::CheckoutStats;
use jj_lib::working_copy::LockedWorkingCopy;
use jj_lib::working_copy::ResetError;
use jj_lib::working_copy::SnapshotError;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::SnapshotStats;
use jj_lib::working_copy::WorkingCopy;
use jj_lib::working_copy::WorkingCopyFactory;
use jj_lib::working_copy::WorkingCopyStateError;
use jj_lib::workspace::WorkingCopyFactories;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceInitError;

#[derive(clap::Parser, Clone, Debug)]
enum CustomCommand {
    /// Initialize a workspace using the "conflicts" working copy
    InitConflicts,
}

fn run_custom_command(
    _ui: &mut Ui,
    command_helper: &CommandHelper,
    command: CustomCommand,
) -> Result<(), CommandError> {
    match command {
        CustomCommand::InitConflicts => {
            let wc_path = command_helper.cwd();
            let settings = command_helper.settings_for_new_workspace(wc_path)?;
            let backend_initializer = |settings: &UserSettings, store_path: &Path| {
                let backend: Box<dyn Backend> =
                    Box::new(GitBackend::init_internal(settings, store_path)?);
                Ok(backend)
            };
            Workspace::init_with_factories(
                &settings,
                wc_path,
                &backend_initializer,
                Signer::from_settings(&settings).map_err(WorkspaceInitError::SignInit)?,
                &ReadonlyRepo::default_op_store_initializer(),
                &ReadonlyRepo::default_op_heads_store_initializer(),
                &ReadonlyRepo::default_index_store_initializer(),
                &ReadonlyRepo::default_submodule_store_initializer(),
                &ConflictsWorkingCopyFactory {},
                WorkspaceName::DEFAULT.to_owned(),
            )?;
            Ok(())
        }
    }
}

fn main() -> std::process::ExitCode {
    let mut working_copy_factories = WorkingCopyFactories::new();
    working_copy_factories.insert(
        ConflictsWorkingCopy::name().to_owned(),
        Box::new(ConflictsWorkingCopyFactory {}),
    );
    CliRunner::init()
        .add_working_copy_factories(working_copy_factories)
        .add_subcommand(run_custom_command)
        .run()
        .into()
}

/// A working copy that adds a .conflicts file with a list of unresolved
/// conflicts.
///
/// Most functions below just delegate to the inner working-copy backend. The
/// only interesting functions are `snapshot()` and `check_out()`. The former
/// adds `.conflicts` to the .gitignores. The latter writes the `.conflicts`
/// file to the working copy.
struct ConflictsWorkingCopy {
    inner: Box<dyn WorkingCopy>,
    working_copy_path: PathBuf,
}

impl ConflictsWorkingCopy {
    fn name() -> &'static str {
        "conflicts"
    }

    fn init(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        operation_id: OperationId,
        workspace_name: WorkspaceNameBuf,
        user_settings: &UserSettings,
    ) -> Result<Self, WorkingCopyStateError> {
        let inner = LocalWorkingCopy::init(
            store,
            working_copy_path.clone(),
            state_path,
            operation_id,
            workspace_name,
            user_settings,
        )?;
        Ok(Self {
            inner: Box::new(inner),
            working_copy_path,
        })
    }

    fn load(
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        user_settings: &UserSettings,
    ) -> Result<Self, WorkingCopyStateError> {
        let inner =
            LocalWorkingCopy::load(store, working_copy_path.clone(), state_path, user_settings)?;
        Ok(Self {
            inner: Box::new(inner),
            working_copy_path,
        })
    }
}

impl WorkingCopy for ConflictsWorkingCopy {
    fn name(&self) -> &str {
        Self::name()
    }

    fn workspace_name(&self) -> &WorkspaceName {
        self.inner.workspace_name()
    }

    fn operation_id(&self) -> &OperationId {
        self.inner.operation_id()
    }

    fn tree(&self) -> Result<&MergedTree, WorkingCopyStateError> {
        self.inner.tree()
    }

    fn sparse_patterns(&self) -> Result<&[RepoPathBuf], WorkingCopyStateError> {
        self.inner.sparse_patterns()
    }

    fn start_mutation(&self) -> Result<Box<dyn LockedWorkingCopy>, WorkingCopyStateError> {
        let inner = self.inner.start_mutation()?;
        Ok(Box::new(LockedConflictsWorkingCopy {
            wc_path: self.working_copy_path.clone(),
            inner,
        }))
    }
}

struct ConflictsWorkingCopyFactory {}

impl WorkingCopyFactory for ConflictsWorkingCopyFactory {
    fn init_working_copy(
        &self,
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        operation_id: OperationId,
        workspace_name: WorkspaceNameBuf,
        settings: &UserSettings,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError> {
        Ok(Box::new(ConflictsWorkingCopy::init(
            store,
            working_copy_path,
            state_path,
            operation_id,
            workspace_name,
            settings,
        )?))
    }

    fn load_working_copy(
        &self,
        store: Arc<Store>,
        working_copy_path: PathBuf,
        state_path: PathBuf,
        settings: &UserSettings,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError> {
        Ok(Box::new(ConflictsWorkingCopy::load(
            store,
            working_copy_path,
            state_path,
            settings,
        )?))
    }
}

struct LockedConflictsWorkingCopy {
    wc_path: PathBuf,
    inner: Box<dyn LockedWorkingCopy + Send>,
}

#[async_trait]
impl LockedWorkingCopy for LockedConflictsWorkingCopy {
    fn old_operation_id(&self) -> &OperationId {
        self.inner.old_operation_id()
    }

    fn old_tree(&self) -> &MergedTree {
        self.inner.old_tree()
    }

    async fn snapshot(
        &mut self,
        options: &SnapshotOptions,
    ) -> Result<(MergedTree, SnapshotStats), SnapshotError> {
        let options = SnapshotOptions {
            base_ignores: options.base_ignores.chain(
                "",
                Path::new(""),
                "/.conflicts".as_bytes(),
            )?,
            ..options.clone()
        };
        self.inner.snapshot(&options).await
    }

    async fn check_out(&mut self, commit: &Commit) -> Result<CheckoutStats, CheckoutError> {
        let conflicts = commit
            .tree()
            .conflicts()
            .map(|(path, _value)| format!("{}\n", path.as_internal_file_string()))
            .join("");
        std::fs::write(self.wc_path.join(".conflicts"), conflicts).unwrap();
        self.inner.check_out(commit).await
    }

    fn rename_workspace(&mut self, new_name: WorkspaceNameBuf) {
        self.inner.rename_workspace(new_name);
    }

    async fn reset(&mut self, commit: &Commit) -> Result<(), ResetError> {
        self.inner.reset(commit).await
    }

    async fn recover(&mut self, commit: &Commit) -> Result<(), ResetError> {
        self.inner.recover(commit).await
    }

    fn sparse_patterns(&self) -> Result<&[RepoPathBuf], WorkingCopyStateError> {
        self.inner.sparse_patterns()
    }

    async fn set_sparse_patterns(
        &mut self,
        new_sparse_patterns: Vec<RepoPathBuf>,
    ) -> Result<CheckoutStats, CheckoutError> {
        self.inner.set_sparse_patterns(new_sparse_patterns).await
    }

    async fn finish(
        self: Box<Self>,
        operation_id: OperationId,
    ) -> Result<Box<dyn WorkingCopy>, WorkingCopyStateError> {
        let inner = self.inner.finish(operation_id).await?;
        Ok(Box::new(ConflictsWorkingCopy {
            inner,
            working_copy_path: self.wc_path,
        }))
    }
}
