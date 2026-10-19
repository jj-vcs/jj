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

//! A wrapper around the [`Backend`] that provides convenient types and caching.

use std::fmt::Debug;
use std::fmt::Formatter;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;

use futures::AsyncRead;
use futures::stream::BoxStream;
use pollster::FutureExt as _;

use crate::backend;
use crate::backend::Backend;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::backend::CopyRecord;
use crate::backend::FileId;
use crate::backend::SigningFn;
use crate::backend::SymlinkId;
use crate::backend::TreeId;
use crate::cached_loader::CachedLoader;
use crate::commit::Commit;
use crate::files::MergeOptions;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::signing::Signer;
use crate::tree::Tree;

// There are more tree objects than commits, and trees are often shared across
// commits.
pub(crate) const COMMIT_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(100).unwrap();
const TREE_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(1000).unwrap();

/// Wraps the low-level backend and makes it return more convenient types. Also
/// adds caching.
pub struct Store {
    backend: Box<dyn Backend>,
    signer: Signer,
    // Concurrent cache misses for the same object share a single backend read.
    commit_cache: CachedLoader<CommitId, Arc<backend::Commit>>,
    tree_cache: CachedLoader<(RepoPathBuf, TreeId), Arc<backend::Tree>>,
    merge_options: MergeOptions,
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Store")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Creates a store wrapping `backend`.
    pub fn new(
        backend: Box<dyn Backend>,
        signer: Signer,
        merge_options: MergeOptions,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            signer,
            commit_cache: CachedLoader::new(COMMIT_CACHE_CAPACITY),
            tree_cache: CachedLoader::new(TREE_CACHE_CAPACITY),
            merge_options,
        })
    }

    /// The underlying backend.
    pub fn backend(&self) -> &dyn Backend {
        self.backend.as_ref()
    }

    /// Returns backend as the implementation type.
    pub fn backend_impl<T: Backend>(&self) -> Option<&T> {
        self.backend.downcast_ref()
    }

    /// The signer used for signing and verifying commits.
    pub fn signer(&self) -> &Signer {
        &self.signer
    }

    /// Default merge options to be used when resolving parent trees.
    pub fn merge_options(&self) -> &MergeOptions {
        &self.merge_options
    }

    /// Returns the copy records between `root` and `head`. See
    /// [`Backend::get_copy_records()`].
    pub fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root: &CommitId,
        head: &CommitId,
    ) -> BackendResult<BoxStream<'_, BackendResult<CopyRecord>>> {
        self.backend.get_copy_records(paths, root, head)
    }

    /// The length of commit IDs in bytes.
    pub fn commit_id_length(&self) -> usize {
        self.backend.commit_id_length()
    }

    /// The length of change IDs in bytes.
    pub fn change_id_length(&self) -> usize {
        self.backend.change_id_length()
    }

    /// The root commit's ID.
    pub fn root_commit_id(&self) -> &CommitId {
        self.backend.root_commit_id()
    }

    /// The root commit's change ID.
    pub fn root_change_id(&self) -> &ChangeId {
        self.backend.root_change_id()
    }

    /// The empty tree's ID.
    pub fn empty_tree_id(&self) -> &TreeId {
        self.backend.empty_tree_id()
    }

    /// The number of concurrent requests the backend can handle well. See
    /// [`Backend::concurrency()`].
    pub fn concurrency(&self) -> usize {
        self.backend.concurrency()
    }

    /// Returns a resolved empty tree.
    pub fn empty_merged_tree(self: &Arc<Self>) -> MergedTree {
        let empty_tree_id = self.backend.empty_tree_id().clone();
        MergedTree::resolved(self.clone(), empty_tree_id)
    }

    /// Returns the ID of a resolved empty tree.
    pub fn empty_merged_tree_id(&self) -> Merge<TreeId> {
        Merge::resolved(self.backend.empty_tree_id().clone())
    }

    /// Returns the root commit.
    pub fn root_commit(self: &Arc<Self>) -> Commit {
        self.get_commit(self.backend.root_commit_id()).unwrap()
    }

    /// Reads a commit, blocking until it's loaded. Prefer
    /// [`Self::get_commit_async()`] in new code.
    pub fn get_commit(self: &Arc<Self>, id: &CommitId) -> BackendResult<Commit> {
        self.get_commit_async(id).block_on()
    }

    /// Reads a commit, using the cache if possible.
    pub async fn get_commit_async(self: &Arc<Self>, id: &CommitId) -> BackendResult<Commit> {
        let data = self.get_backend_commit(id).await?;
        Ok(Commit::new(self.clone(), id.clone(), data))
    }

    async fn get_backend_commit(
        self: &Arc<Self>,
        id: &CommitId,
    ) -> BackendResult<Arc<backend::Commit>> {
        self.commit_cache
            .get_or_load(id, || {
                let store = self.clone();
                let id = id.clone();
                async move {
                    let commit = store.backend.read_commit(&id).await?;
                    Ok(Arc::new(commit))
                }
            })
            .await
    }

    /// Writes a commit to the backend, optionally signing it. The commit must
    /// have at least one parent.
    pub async fn write_commit(
        self: &Arc<Self>,
        commit: backend::Commit,
        sign_with: Option<&mut SigningFn<'_>>,
    ) -> BackendResult<Commit> {
        assert!(!commit.parents.is_empty());

        let (commit_id, commit) = self.backend.write_commit(commit, sign_with).await?;
        let data = Arc::new(commit);
        self.commit_cache.insert(commit_id.clone(), data.clone());

        Ok(Commit::new(self.clone(), commit_id, data))
    }

    /// Reads the tree at directory `dir`, using the cache if possible.
    pub async fn get_tree(self: &Arc<Self>, dir: RepoPathBuf, id: &TreeId) -> BackendResult<Tree> {
        let data = self.get_backend_tree(&dir, id).await?;
        Ok(Tree::new(self.clone(), dir, id.clone(), data))
    }

    async fn get_backend_tree(
        self: &Arc<Self>,
        dir: &RepoPath,
        id: &TreeId,
    ) -> BackendResult<Arc<backend::Tree>> {
        let key = (dir.to_owned(), id.clone());
        self.tree_cache
            .get_or_load(&key, || {
                let store = self.clone();
                let key = key.clone();
                async move {
                    let (dir, id) = &key;
                    let data = store.backend.read_tree(dir, id).await?;
                    Ok(Arc::new(data))
                }
            })
            .await
    }

    /// Writes a tree for the directory at `path` to the backend.
    pub async fn write_tree(
        self: &Arc<Self>,
        path: &RepoPath,
        tree: backend::Tree,
    ) -> BackendResult<Tree> {
        let tree_id = self.backend.write_tree(path, &tree).await?;
        let data = Arc::new(tree);
        self.tree_cache
            .insert((path.to_owned(), tree_id.clone()), data.clone());

        Ok(Tree::new(self.clone(), path.to_owned(), tree_id, data))
    }

    /// Returns a reader for the contents of a file.
    pub async fn read_file(
        &self,
        path: &RepoPath,
        id: &FileId,
    ) -> BackendResult<Pin<Box<dyn AsyncRead + Send>>> {
        self.backend.read_file(path, id).await
    }

    /// Writes the contents of a file to the backend.
    pub async fn write_file(
        &self,
        path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> BackendResult<FileId> {
        self.backend.write_file(path, contents).await
    }

    /// Reads the target of a symlink.
    pub async fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> BackendResult<String> {
        self.backend.read_symlink(path, id).await
    }

    /// Writes a symlink with the given target to the backend.
    pub async fn write_symlink(&self, path: &RepoPath, contents: &str) -> BackendResult<SymlinkId> {
        self.backend.write_symlink(path, contents).await
    }

    /// Clear cached objects. Mainly intended for testing.
    pub fn clear_caches(&self) {
        self.commit_cache.clear();
        self.tree_cache.clear();
    }
}
