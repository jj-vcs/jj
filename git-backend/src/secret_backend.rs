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

//! Provides a backend for testing ACLs

use std::path::Path;
use std::pin::Pin;
use std::time::SystemTime;

use async_trait::async_trait;
use futures::AsyncRead;
use futures::stream::BoxStream;
use jj_core::commit_store::ChangeId;
use jj_core::commit_store::Commit;
use jj_core::commit_store::CommitId;
use jj_core::commit_store::CommitStore;
use jj_core::commit_store::CommitStoreError;
use jj_core::commit_store::CommitStoreLoadError;
use jj_core::commit_store::CommitStoreResult;
use jj_core::commit_store::CopyHistory;
use jj_core::commit_store::CopyId;
use jj_core::commit_store::CopyRecord;
use jj_core::commit_store::FileId;
use jj_core::commit_store::RelatedCopy;
use jj_core::commit_store::SigningFn;
use jj_core::commit_store::SymlinkId;
use jj_core::commit_store::Tree;
use jj_core::commit_store::TreeId;
use jj_core::object_id::ObjectId as _;
use jj_core::repo_path::RepoPath;
use jj_core::repo_path::RepoPathBuf;

use crate::git_backend::GitBackendSettings;
use crate::git_backend::GitCommitStore;

const SECRET_CONTENTS_HEX: [&str; 4] = [
    // SHA-1
    "d97c5eada5d8c52079031eef0107a4430a9617c5", // "secret\n"
    "536aca34dbae6b2b8af26bebdcba83543c9546f0", // "secret"
    // SHA-256
    "6ecf416550a9e53369aebefa2071a4de6e03ed9a10136e60c3fcbd3bd05897fc", // "secret\n"
    "ed23037c9393c06980d673c327ad1b6719aec4ba614e1a34544d0cabe35f8be8", // "secret"
];

/// A commit store that's completely compatible with the Git backend, except
/// that it refuses to read files and symlinks with the word "secret" in the
/// path, or "secret" or "secret\n" in the content.
#[derive(Debug)]
pub struct SecretCommitStore {
    inner: GitCommitStore,
}

impl SecretCommitStore {
    /// "secret"
    pub const NAME: &str = "secret";

    /// Loads the backend from the given path.
    pub fn load(
        settings: &GitBackendSettings,
        store_path: &Path,
    ) -> Result<Self, CommitStoreLoadError> {
        let inner = GitCommitStore::load(settings, store_path)?;
        Ok(Self { inner })
    }

    /// Convert a git repo to using `SecretCommitStore`
    // TODO: Avoid this hack
    pub fn adopt_git_repo(workspace_path: &Path) {
        std::fs::write(
            workspace_path
                .join(".jj")
                .join("repo")
                .join("store")
                .join("type"),
            Self::NAME,
        )
        .unwrap();
    }
}

#[async_trait]
impl CommitStore for SecretCommitStore {
    fn name(&self) -> &str {
        Self::NAME
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
        if path.as_internal_file_string().contains("secret")
            || SECRET_CONTENTS_HEX.contains(&id.hex().as_ref())
        {
            return Err(CommitStoreError::ReadAccessDenied {
                object_type: "file".to_string(),
                hash: id.hex(),
                source: "No access".into(),
            });
        }
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
        if path.as_internal_file_string().contains("secret")
            || SECRET_CONTENTS_HEX.contains(&id.hex().as_ref())
        {
            return Err(CommitStoreError::ReadAccessDenied {
                object_type: "symlink".to_string(),
                hash: id.hex(),
                source: "No access".into(),
            });
        }
        self.inner.read_symlink(path, id).await
    }

    async fn write_symlink(&self, path: &RepoPath, target: &str) -> CommitStoreResult<SymlinkId> {
        self.inner.write_symlink(path, target).await
    }

    async fn read_copy(&self, _id: &CopyId) -> CommitStoreResult<CopyHistory> {
        Err(CommitStoreError::Unsupported(
            "The secret backend doesn't support copies".to_string(),
        ))
    }

    async fn write_copy(&self, _contents: &CopyHistory) -> CommitStoreResult<CopyId> {
        Err(CommitStoreError::Unsupported(
            "The secret backend doesn't support copies".to_string(),
        ))
    }

    async fn get_related_copies(&self, _copy_id: &CopyId) -> CommitStoreResult<Vec<RelatedCopy>> {
        Err(CommitStoreError::Unsupported(
            "The secret backend doesn't support copies".to_string(),
        ))
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
