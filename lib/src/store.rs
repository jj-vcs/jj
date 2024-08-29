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

#![allow(missing_docs)]

use std::any::Any;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::io::Read;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

use clru::CLruCache;
use futures::stream::BoxStream;
use pollster::FutureExt;

use crate::backend;
use crate::backend::Backend;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::CommitId;
use crate::backend::ConflictId;
use crate::backend::CopyRecord;
use crate::backend::FileId;
use crate::backend::MergedTreeId;
use crate::backend::SigningFn;
use crate::backend::SymlinkId;
use crate::backend::TreeId;
use crate::commit::Commit;
use crate::index::Index;
use crate::merge::Merge;
use crate::merge::MergedTreeValue;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::signing::Signer;
use crate::tree::Tree;
use crate::tree_builder::TreeBuilder;

// There are more tree objects than commits, and trees are often shared across
// commits.
const COMMIT_CACHE_CAPACITY: usize = 100;
const TREE_CACHE_CAPACITY: usize = 1000;

/// Wraps the low-level backend and makes it return more convenient types. Also
/// adds caching.
pub struct Store {
    backend: Box<dyn Backend>,
    signer: Signer,
    commit_cache: Mutex<CLruCache<CommitId, Arc<backend::Commit>>>,
    tree_cache: Mutex<CLruCache<(RepoPathBuf, TreeId), Arc<backend::Tree>>>,
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Store")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl Store {
    pub fn new(backend: Box<dyn Backend>, signer: Signer) -> Arc<Self> {
        Arc::new(Store {
            backend,
            signer,
            commit_cache: Mutex::new(CLruCache::new(COMMIT_CACHE_CAPACITY.try_into().unwrap())),
            tree_cache: Mutex::new(CLruCache::new(TREE_CACHE_CAPACITY.try_into().unwrap())),
        })
    }

    pub fn backend_impl(&self) -> &dyn Any {
        self.backend.as_any()
    }

    pub fn signer(&self) -> &Signer {
        &self.signer
    }

    pub fn get_copy_records(
        &self,
        paths: Option<&[RepoPathBuf]>,
        root: &CommitId,
        head: &CommitId,
    ) -> BackendResult<BoxStream<BackendResult<CopyRecord>>> {
        self.backend.get_copy_records(paths, root, head)
    }

    pub fn commit_id_length(&self) -> usize {
        self.backend.commit_id_length()
    }

    pub fn change_id_length(&self) -> usize {
        self.backend.change_id_length()
    }

    pub fn root_commit_id(&self) -> &CommitId {
        self.backend.root_commit_id()
    }

    pub fn root_change_id(&self) -> &ChangeId {
        self.backend.root_change_id()
    }

    pub fn empty_tree_id(&self) -> &TreeId {
        self.backend.empty_tree_id()
    }

    pub fn concurrency(&self) -> usize {
        self.backend.concurrency()
    }

    pub fn empty_merged_tree_id(&self) -> MergedTreeId {
        MergedTreeId::resolved(self.backend.empty_tree_id().clone())
    }

    pub fn root_commit(self: &Arc<Self>) -> Commit {
        self.get_commit(self.backend.root_commit_id()).unwrap()
    }

    pub fn get_commit(self: &Arc<Self>, id: &CommitId) -> BackendResult<Commit> {
        self.get_commit_async(id).block_on()
    }

    pub async fn get_commit_async(self: &Arc<Self>, id: &CommitId) -> BackendResult<Commit> {
        let data = self.get_backend_commit(id).await?;
        Ok(Commit::new(self.clone(), id.clone(), data))
    }

    async fn get_backend_commit(&self, id: &CommitId) -> BackendResult<Arc<backend::Commit>> {
        {
            let mut locked_cache = self.commit_cache.lock().unwrap();
            if let Some(data) = locked_cache.get(id).cloned() {
                return Ok(data);
            }
        }
        let commit = self.backend.read_commit(id).await?;
        let data = Arc::new(commit);
        let mut locked_cache = self.commit_cache.lock().unwrap();
        locked_cache.put(id.clone(), data.clone());
        Ok(data)
    }

    pub fn write_commit(
        self: &Arc<Self>,
        commit: backend::Commit,
        sign_with: Option<&mut SigningFn>,
    ) -> BackendResult<Commit> {
        assert!(!commit.parents.is_empty());

        let (commit_id, commit) = self.backend.write_commit(commit, sign_with)?;
        let data = Arc::new(commit);
        {
            let mut locked_cache = self.commit_cache.lock().unwrap();
            locked_cache.put(commit_id.clone(), data.clone());
        }

        Ok(Commit::new(self.clone(), commit_id, data))
    }

    pub fn get_tree(self: &Arc<Self>, dir: &RepoPath, id: &TreeId) -> BackendResult<Tree> {
        self.get_tree_async(dir, id).block_on()
    }

    pub async fn get_tree_async(
        self: &Arc<Self>,
        dir: &RepoPath,
        id: &TreeId,
    ) -> BackendResult<Tree> {
        let data = self.get_backend_tree(dir, id).await?;
        Ok(Tree::new(self.clone(), dir.to_owned(), id.clone(), data))
    }

    async fn get_backend_tree(
        &self,
        dir: &RepoPath,
        id: &TreeId,
    ) -> BackendResult<Arc<backend::Tree>> {
        let key = (dir.to_owned(), id.clone());
        {
            let mut locked_cache = self.tree_cache.lock().unwrap();
            if let Some(data) = locked_cache.get(&key).cloned() {
                return Ok(data);
            }
        }
        let data = self.backend.read_tree(dir, id).await?;
        let data = Arc::new(data);
        let mut locked_cache = self.tree_cache.lock().unwrap();
        locked_cache.put(key, data.clone());
        Ok(data)
    }

    pub fn get_root_tree(self: &Arc<Self>, id: &MergedTreeId) -> BackendResult<MergedTree> {
        match &id {
            MergedTreeId::Legacy(id) => {
                let tree = self.get_tree(RepoPath::root(), id)?;
                MergedTree::from_legacy_tree(tree)
            }
            MergedTreeId::Merge(ids) => {
                let trees = ids.try_map(|id| self.get_tree(RepoPath::root(), id))?;
                Ok(MergedTree::new(trees))
            }
        }
    }

    pub fn write_tree(
        self: &Arc<Self>,
        path: &RepoPath,
        tree: backend::Tree,
    ) -> BackendResult<Tree> {
        let tree_id = self.backend.write_tree(path, &tree)?;
        let data = Arc::new(tree);
        {
            let mut locked_cache = self.tree_cache.lock().unwrap();
            locked_cache.put((path.to_owned(), tree_id.clone()), data.clone());
        }

        Ok(Tree::new(self.clone(), path.to_owned(), tree_id, data))
    }

    pub fn read_file(&self, path: &RepoPath, id: &FileId) -> BackendResult<Box<dyn Read>> {
        self.read_file_async(path, id).block_on()
    }

    pub async fn read_file_async(
        &self,
        path: &RepoPath,
        id: &FileId,
    ) -> BackendResult<Box<dyn Read>> {
        self.backend.read_file(path, id).await
    }

    pub fn write_file(&self, path: &RepoPath, contents: &mut dyn Read) -> BackendResult<FileId> {
        self.backend.write_file(path, contents)
    }

    pub fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> BackendResult<String> {
        self.read_symlink_async(path, id).block_on()
    }

    pub async fn read_symlink_async(
        &self,
        path: &RepoPath,
        id: &SymlinkId,
    ) -> BackendResult<String> {
        self.backend.read_symlink(path, id).await
    }

    pub fn write_symlink(&self, path: &RepoPath, contents: &str) -> BackendResult<SymlinkId> {
        self.backend.write_symlink(path, contents)
    }

    pub fn read_conflict(
        &self,
        path: &RepoPath,
        id: &ConflictId,
    ) -> BackendResult<MergedTreeValue> {
        let backend_conflict = self.backend.read_conflict(path, id)?;
        Ok(Merge::from_backend_conflict(backend_conflict))
    }

    pub fn write_conflict(
        &self,
        path: &RepoPath,
        contents: &MergedTreeValue,
    ) -> BackendResult<ConflictId> {
        self.backend
            .write_conflict(path, &contents.clone().into_backend_conflict())
    }

    pub fn tree_builder(self: &Arc<Self>, base_tree_id: TreeId) -> TreeBuilder {
        TreeBuilder::new(self.clone(), base_tree_id)
    }

    pub fn gc(&self, index: &dyn Index, keep_newer: SystemTime) -> BackendResult<()> {
        self.backend.gc(index, keep_newer)
    }
}
