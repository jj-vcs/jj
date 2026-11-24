// Copyright 2022 The Jujutsu Authors
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
use std::pin::Pin;
use std::time::SystemTime;

use async_trait::async_trait;
use futures::AsyncRead;
use futures::stream::BoxStream;
use jj_cli::cli_util::CliRunner;
use jj_cli::cli_util::CommandHelper;
use jj_cli::command_error::CommandError;
use jj_cli::ui::Ui;
use jj_lib::commit_store::ChangeId;
use jj_lib::commit_store::Commit;
use jj_lib::commit_store::CommitId;
use jj_lib::commit_store::CommitStore;
use jj_lib::commit_store::CommitStoreInitError;
use jj_lib::commit_store::CommitStoreLoadError;
use jj_lib::commit_store::CommitStoreResult;
use jj_lib::commit_store::CopyHistory;
use jj_lib::commit_store::CopyId;
use jj_lib::commit_store::CopyRecord;
use jj_lib::commit_store::FileId;
use jj_lib::commit_store::RelatedCopy;
use jj_lib::commit_store::SigningFn;
use jj_lib::commit_store::SymlinkId;
use jj_lib::commit_store::Tree;
use jj_lib::commit_store::TreeId;
use jj_lib::git_backend::GitCommitStore;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::settings::UserSettings;
use jj_lib::signing_factory::signer_from_settings;
use jj_lib::store_factories::StoreFactories;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceInitError;

#[derive(clap::Parser, Clone, Debug)]
enum CustomCommand {
    /// Initialize a workspace using the Jit backend
    InitJit,
}

fn create_store_factories() -> StoreFactories {
    let mut store_factories = StoreFactories::empty();
    // Register the backend so it can be loaded when the repo is loaded. The name
    // must match `CommitStore::name()`.
    store_factories.add_commit_store(
        "jit",
        Box::new(|settings, store_path| Ok(Box::new(JitCommitStore::load(settings, store_path)?))),
    );
    store_factories
}

async fn run_custom_command(
    ui: &mut Ui,
    command_helper: &CommandHelper,
    command: CustomCommand,
) -> Result<(), CommandError> {
    match command {
        CustomCommand::InitJit => {
            let wc_path = command_helper.cwd();
            let settings = command_helper.settings_for_new_workspace(ui, wc_path)?.0;
            // Initialize a workspace with the custom backend
            Workspace::init_with_commit_store(
                &settings,
                wc_path,
                WorkspaceName::DEFAULT,
                &|settings, store_path| Ok(Box::new(JitCommitStore::init(settings, store_path)?)),
                signer_from_settings(&settings).map_err(WorkspaceInitError::SignInit)?,
            )
            .await?;
            Ok(())
        }
    }
}

fn main() -> std::process::ExitCode {
    CliRunner::init()
        .add_store_factories(create_store_factories())
        .add_subcommand(run_custom_command)
        .run()
        .into()
}

/// A commit backend that's extremely similar to the Git backend
#[derive(Debug)]
struct JitCommitStore {
    inner: GitCommitStore,
}

impl JitCommitStore {
    fn init(settings: &UserSettings, store_path: &Path) -> Result<Self, CommitStoreInitError> {
        let git_settings = settings
            .git_backend_settings()
            .map_err(CommitStoreInitError::new)?;
        let inner =
            GitCommitStore::init_internal(&git_settings, store_path, gix::hash::Kind::default())?;
        Ok(Self { inner })
    }

    fn load(settings: &UserSettings, store_path: &Path) -> Result<Self, CommitStoreLoadError> {
        let git_settings = settings
            .git_backend_settings()
            .map_err(CommitStoreLoadError::new)?;
        let inner = GitCommitStore::load(&git_settings, store_path)?;
        Ok(Self { inner })
    }
}

#[async_trait]
impl CommitStore for JitCommitStore {
    fn name(&self) -> &'static str {
        "jit"
    }

    fn commit_id_length(&self) -> usize {
        self.inner.commit_id_length()
    }

    fn change_id_length(&self) -> usize {
        self.inner.change_id_length()
    }

    fn root_commit_id(&self) -> &CommitId {
        self.inner.root_commit_id()
    }

    fn root_change_id(&self) -> &ChangeId {
        self.inner.root_change_id()
    }

    fn empty_tree_id(&self) -> &TreeId {
        self.inner.empty_tree_id()
    }

    fn concurrency(&self) -> usize {
        1
    }

    async fn read_file(
        &self,
        path: &RepoPath,
        id: &FileId,
    ) -> CommitStoreResult<Pin<Box<dyn AsyncRead + Send>>> {
        self.inner.read_file(path, id).await
    }

    async fn write_file(
        &self,
        path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> CommitStoreResult<FileId> {
        self.inner.write_file(path, contents).await
    }

    async fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> CommitStoreResult<String> {
        self.inner.read_symlink(path, id).await
    }

    async fn write_symlink(&self, path: &RepoPath, target: &str) -> CommitStoreResult<SymlinkId> {
        self.inner.write_symlink(path, target).await
    }

    async fn read_copy(&self, id: &CopyId) -> CommitStoreResult<CopyHistory> {
        self.inner.read_copy(id).await
    }

    async fn write_copy(&self, contents: &CopyHistory) -> CommitStoreResult<CopyId> {
        self.inner.write_copy(contents).await
    }

    async fn get_related_copies(&self, copy_id: &CopyId) -> CommitStoreResult<Vec<RelatedCopy>> {
        self.inner.get_related_copies(copy_id).await
    }

    async fn read_tree(&self, path: &RepoPath, id: &TreeId) -> CommitStoreResult<Tree> {
        self.inner.read_tree(path, id).await
    }

    async fn write_tree(&self, path: &RepoPath, contents: &Tree) -> CommitStoreResult<TreeId> {
        self.inner.write_tree(path, contents).await
    }

    async fn read_commit(&self, id: &CommitId) -> CommitStoreResult<Commit> {
        self.inner.read_commit(id).await
    }

    async fn write_commit(
        &self,
        contents: Commit,
        sign_with: Option<&mut SigningFn>,
    ) -> CommitStoreResult<(CommitId, Commit)> {
        self.inner.write_commit(contents, sign_with).await
    }

    fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root: &CommitId,
        head: &CommitId,
    ) -> CommitStoreResult<BoxStream<'_, CommitStoreResult<CopyRecord>>> {
        self.inner.get_copy_records(paths, root, head)
    }

    async fn gc(&self, heads: &[CommitId], keep_newer: SystemTime) -> CommitStoreResult<()> {
        self.inner.gc(heads, keep_newer).await
    }
}
