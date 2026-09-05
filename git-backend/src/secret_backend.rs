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
use jj_core::backend::Backend;
use jj_core::backend::BackendError;
use jj_core::backend::BackendLoadError;
use jj_core::backend::BackendResult;
use jj_core::backend::ChangeId;
use jj_core::backend::Commit;
use jj_core::backend::CommitId;
use jj_core::backend::CopyHistory;
use jj_core::backend::CopyId;
use jj_core::backend::CopyRecord;
use jj_core::backend::FileId;
use jj_core::backend::RelatedCopy;
use jj_core::backend::SigningFn;
use jj_core::backend::SymlinkId;
use jj_core::backend::Tree;
use jj_core::backend::TreeId;
use jj_core::object_id::ObjectId as _;
use jj_core::repo_path::RepoPath;
use jj_core::repo_path::RepoPathBuf;

use crate::git_backend::GitBackend;
use crate::git_backend::GitBackendSettings;

const SECRET_CONTENTS_HEX: [&str; 2] = [
    "d97c5eada5d8c52079031eef0107a4430a9617c5", // "secret\n"
    "536aca34dbae6b2b8af26bebdcba83543c9546f0", // "secret"
];

/// A commit backend that's completely compatible with the Git backend, except
/// that it refuses to read files and symlinks with the word "secret" in the
/// path, or "secret" or "secret\n" in the content.
#[derive(Debug)]
pub struct SecretBackend {
    inner: GitBackend,
}

impl SecretBackend {
    /// "secret"
    pub const NAME: &str = "secret";

    /// Loads the backend from the given path.
    pub fn load(
        settings: &GitBackendSettings,
        store_path: &Path,
    ) -> Result<Self, BackendLoadError> {
        let inner = GitBackend::load(settings, store_path)?;
        Ok(Self { inner })
    }

    /// Convert a git repo to using `SecretBackend`
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
impl Backend for SecretBackend {
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
    ) -> BackendResult<Pin<Box<dyn AsyncRead + Send>>> {
        if path.as_internal_file_string().contains("secret")
            || SECRET_CONTENTS_HEX.contains(&id.hex().as_ref())
        {
            return Err(BackendError::ReadAccessDenied {
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
    ) -> BackendResult<FileId> {
        self.inner.write_file(path, contents).await
    }

    async fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> BackendResult<String> {
        if path.as_internal_file_string().contains("secret")
            || SECRET_CONTENTS_HEX.contains(&id.hex().as_ref())
        {
            return Err(BackendError::ReadAccessDenied {
                object_type: "symlink".to_string(),
                hash: id.hex(),
                source: "No access".into(),
            });
        }
        self.inner.read_symlink(path, id).await
    }

    async fn write_symlink(&self, path: &RepoPath, target: &str) -> BackendResult<SymlinkId> {
        self.inner.write_symlink(path, target).await
    }

    async fn read_copy(&self, _id: &CopyId) -> BackendResult<CopyHistory> {
        Err(BackendError::Unsupported(
            "The secret backend doesn't support copies".to_string(),
        ))
    }

    async fn write_copy(&self, _contents: &CopyHistory) -> BackendResult<CopyId> {
        Err(BackendError::Unsupported(
            "The secret backend doesn't support copies".to_string(),
        ))
    }

    async fn get_related_copies(&self, _copy_id: &CopyId) -> BackendResult<Vec<RelatedCopy>> {
        Err(BackendError::Unsupported(
            "The secret backend doesn't support copies".to_string(),
        ))
    }

    async fn read_tree(&self, path: &RepoPath, id: &TreeId) -> BackendResult<Tree> {
        self.inner.read_tree(path, id).await
    }

    async fn write_tree(&self, path: &RepoPath, contents: &Tree) -> BackendResult<TreeId> {
        self.inner.write_tree(path, contents).await
    }

    async fn read_commit(&self, id: &CommitId) -> BackendResult<Commit> {
        self.inner.read_commit(id).await
    }

    async fn write_commit(
        &self,
        contents: Commit,
        sign_with: Option<&mut SigningFn>,
    ) -> BackendResult<(CommitId, Commit)> {
        self.inner.write_commit(contents, sign_with).await
    }

    fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root: &CommitId,
        head: &CommitId,
    ) -> BackendResult<BoxStream<'_, BackendResult<CopyRecord>>> {
        self.inner.get_copy_records(paths, root, head)
    }

    async fn gc(&self, heads: &[CommitId], keep_newer: SystemTime) -> BackendResult<()> {
        self.inner.gc(heads, keep_newer).await
    }
}
