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

//! A wrapper around the [`CommitStore`] that provides convenient types and caching.

use std::fmt::Debug;
use std::fmt::Formatter;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;

use futures::AsyncRead;
use futures::stream::BoxStream;
use pollster::FutureExt as _;

use crate::cached_loader::CachedLoader;
use crate::commit::Commit;
use crate::commit_store;
use crate::commit_store::ChangeId;
use crate::commit_store::CommitId;
use crate::commit_store::CommitStore;
use crate::commit_store::CommitStoreResult;
use crate::commit_store::CopyRecord;
use crate::commit_store::FileId;
use crate::commit_store::SigningFn;
use crate::commit_store::SymlinkId;
use crate::commit_store::TreeId;
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
    commit_store: Box<dyn CommitStore>,
    signer: Signer,
    // Concurrent cache misses for the same object share a single backend read.
    commit_cache: CachedLoader<CommitId, Arc<commit_store::Commit>>,
    tree_cache: CachedLoader<(RepoPathBuf, TreeId), Arc<commit_store::Tree>>,
    merge_options: MergeOptions,
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Store")
            .field("commit_store", &self.commit_store)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Creates a store wrapping `commit_store`.
    pub fn new(
        commit_store: Box<dyn CommitStore>,
        signer: Signer,
        merge_options: MergeOptions,
    ) -> Arc<Self> {
        Arc::new(Self {
            commit_store,
            signer,
            commit_cache: CachedLoader::new(COMMIT_CACHE_CAPACITY),
            tree_cache: CachedLoader::new(TREE_CACHE_CAPACITY),
            merge_options,
        })
    }

    /// The underlying backend.
    pub fn commit_store(&self) -> &dyn CommitStore {
        self.commit_store.as_ref()
    }

    /// Returns backend as the implementation type.
    pub fn commit_store_impl<T: CommitStore>(&self) -> Option<&T> {
        self.commit_store.downcast_ref()
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
    /// [`CommitStore::get_copy_records()`].
    pub fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root: &CommitId,
        head: &CommitId,
    ) -> CommitStoreResult<BoxStream<'_, CommitStoreResult<CopyRecord>>> {
        self.commit_store.get_copy_records(paths, root, head)
    }

    /// The length of commit IDs in bytes.
    pub fn commit_id_length(&self) -> usize {
        self.commit_store.commit_id_length()
    }

    /// The length of change IDs in bytes.
    pub fn change_id_length(&self) -> usize {
        self.commit_store.change_id_length()
    }

    /// The root commit's ID.
    pub fn root_commit_id(&self) -> &CommitId {
        self.commit_store.root_commit_id()
    }

    /// The root commit's change ID.
    pub fn root_change_id(&self) -> &ChangeId {
        self.commit_store.root_change_id()
    }

    /// The empty tree's ID.
    pub fn empty_tree_id(&self) -> &TreeId {
        self.commit_store.empty_tree_id()
    }

    /// The number of concurrent requests the backend can handle well. See
    /// [`CommitStore::concurrency()`].
    pub fn concurrency(&self) -> usize {
        self.commit_store.concurrency()
    }

    /// Returns a resolved empty tree.
    pub fn empty_merged_tree(self: &Arc<Self>) -> MergedTree {
        let empty_tree_id = self.commit_store.empty_tree_id().clone();
        MergedTree::resolved(self.clone(), empty_tree_id)
    }

    /// Returns the ID of a resolved empty tree.
    pub fn empty_merged_tree_id(&self) -> Merge<TreeId> {
        Merge::resolved(self.commit_store.empty_tree_id().clone())
    }

    /// Returns the root commit.
    pub fn root_commit(self: &Arc<Self>) -> Commit {
        self.get_commit(self.commit_store.root_commit_id()).unwrap()
    }

    /// Reads a commit, blocking until it's loaded. Prefer
    /// [`Self::get_commit_async()`] in new code.
    pub fn get_commit(self: &Arc<Self>, id: &CommitId) -> CommitStoreResult<Commit> {
        self.get_commit_async(id).block_on()
    }

    /// Reads a commit, using the cache if possible.
    pub async fn get_commit_async(self: &Arc<Self>, id: &CommitId) -> CommitStoreResult<Commit> {
        let data = self.get_commit_store_commit(id).await?;
        Ok(Commit::new(self.clone(), id.clone(), data))
    }

    async fn get_commit_store_commit(
        self: &Arc<Self>,
        id: &CommitId,
    ) -> CommitStoreResult<Arc<commit_store::Commit>> {
        self.commit_cache
            .get_or_load(id, || {
                let store = self.clone();
                let id = id.clone();
                async move {
                    let commit = store.commit_store.read_commit(&id).await?;
                    Ok(Arc::new(commit))
                }
            })
            .await
    }

    /// Writes a commit to the backend, optionally signing it. The commit must
    /// have at least one parent.
    pub async fn write_commit(
        self: &Arc<Self>,
        commit: commit_store::Commit,
        sign_with: Option<&mut SigningFn<'_>>,
    ) -> CommitStoreResult<Commit> {
        assert!(!commit.parents.is_empty());

        let (commit_id, commit) = self.commit_store.write_commit(commit, sign_with).await?;
        let data = Arc::new(commit);
        self.commit_cache.insert(commit_id.clone(), data.clone());

        Ok(Commit::new(self.clone(), commit_id, data))
    }

    /// Reads the tree at directory `dir`, using the cache if possible.
    pub async fn get_tree(
        self: &Arc<Self>,
        dir: RepoPathBuf,
        id: &TreeId,
    ) -> CommitStoreResult<Tree> {
        let data = self.get_commit_store_tree(&dir, id).await?;
        Ok(Tree::new(self.clone(), dir, id.clone(), data))
    }

    async fn get_commit_store_tree(
        self: &Arc<Self>,
        dir: &RepoPath,
        id: &TreeId,
    ) -> CommitStoreResult<Arc<commit_store::Tree>> {
        let key = (dir.to_owned(), id.clone());
        self.tree_cache
            .get_or_load(&key, || {
                let store = self.clone();
                let key = key.clone();
                async move {
                    let (dir, id) = &key;
                    let data = store.commit_store.read_tree(dir, id).await?;
                    Ok(Arc::new(data))
                }
            })
            .await
    }

    /// Writes a tree for the directory at `path` to the backend.
    pub async fn write_tree(
        self: &Arc<Self>,
        path: &RepoPath,
        tree: commit_store::Tree,
    ) -> CommitStoreResult<Tree> {
        let tree_id = self.commit_store.write_tree(path, &tree).await?;
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
    ) -> CommitStoreResult<Pin<Box<dyn AsyncRead + Send>>> {
        self.commit_store.read_file(path, id).await
    }

    /// Writes the contents of a file to the backend.
    pub async fn write_file(
        &self,
        path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> CommitStoreResult<FileId> {
        self.commit_store.write_file(path, contents).await
    }

    /// Reads the target of a symlink.
    pub async fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> CommitStoreResult<String> {
        self.commit_store.read_symlink(path, id).await
    }

    /// Writes a symlink with the given target to the backend.
    pub async fn write_symlink(
        &self,
        path: &RepoPath,
        contents: &str,
    ) -> CommitStoreResult<SymlinkId> {
        self.commit_store.write_symlink(path, contents).await
    }

    /// Clear cached objects. Mainly intended for testing.
    pub fn clear_caches(&self) {
        self.commit_cache.clear();
        self.tree_cache.clear();
    }
}
